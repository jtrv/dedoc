use std::fs::{create_dir_all, remove_file, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use attohttpc::get;

use serde::de::{Error, MapAccess, Visitor};
use serde::Deserializer;

use toiletcli::flags;
use toiletcli::flags::*;

use crate::common::{
    deserialize_docs_json, get_docset_path, get_flag_error, is_docs_json_exists,
    is_docset_downloaded, is_docset_in_docs_or_print_warning,
};
use crate::common::{Docs, ResultS};
use crate::common::{
    BOLD, DEFAULT_DB_JSON_LINK, DEFAULT_USER_AGENT, GREEN, PROGRAM_NAME, RESET, VERSION,
};
use crate::print_warning;

fn show_download_help() -> ResultS {
    println!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} download{RESET} [-f] <docset1> [docset2, ..]
    Download a docset. Available docsets can be displayed using `list`.

{GREEN}OPTIONS{RESET}
    -f, --force                     Force the download and overwrite files.
        --help                      Display help message."
    );
    Ok(())
}

fn download_db_and_index_json_with_progress(docset_name: &String, docs: &[Docs]) -> ResultS {
    let user_agent = format!("{DEFAULT_USER_AGENT}/{VERSION}");

    for entry in docs.iter() {
        if docset_name == &entry.slug {
            let docset_path = get_docset_path(docset_name)?;

            if !docset_path.exists() {
                create_dir_all(&docset_path).map_err(|err| {
                    format!("Cannot create `{}` directory: {err}", docset_path.display())
                })?;
            }

            for (file_name, i) in [("db.json", 1), ("index.json", 2)] {
                let file_path = docset_path.join(file_name);

                let file = File::create(&file_path)
                    .map_err(|err| format!("Could not create `{}`: {err}", file_path.display()))?;

                let download_link = format!(
                    "{DEFAULT_DB_JSON_LINK}/{docset_name}/{}?{}",
                    file_name, entry.mtime
                );

                let response = get(&download_link)
                    .header_append("user-agent", &user_agent)
                    .send()
                    .map_err(|err| format!("Could not GET {download_link}: {err}"))?;

                let mut file_writer = BufWriter::new(file);
                let mut response_reader = BufReader::new(response);

                let mut buffer = [0; 1024 * 4];
                let mut file_size = 0;

                while let Ok(size) = response_reader.read(&mut buffer) {
                    if size == 0 {
                        break;
                    }

                    file_writer
                        .write(&buffer[..size])
                        .map_err(|err| format!("Could not download file: {err}"))?;

                    file_size += size;

                    print!("\rReceived {file_size} bytes, file {i} of 2...");
                }
                println!();
            }
        }
    }

    Ok(())
}

// Remove class="...", title="...", data-language="..." attributes from HTML tags to reduce size.
fn sanitize_html_line(html_line: String) -> String {
    enum State {
        Default,
        InTag,
        InKey,
        InValue,
    }

    let length = html_line.len();
    let bytes = html_line.as_bytes();

    let mut sanitized_line = String::new();

    let mut state = State::Default;
    let mut position = 0;

    let html_line_chars = html_line.chars();

    for ch in html_line_chars {
        match state {
            State::Default => {
                if ch == '<' {
                    state = State::InTag;
                }
                sanitized_line.push(ch);
            }
            State::InTag => match ch {
                'd' if position + 15 < length
                    && bytes[position..position + 15] == *b"data-language=\"" =>
                {
                    state = State::InKey;
                }
                't' if position + 7 < length && bytes[position..position + 7] == *b"title=\"" => {
                    state = State::InKey;
                }
                'c' if position + 7 < length && bytes[position..position + 7] == *b"class=\"" => {
                    state = State::InKey;
                }
                '>' => {
                    state = State::Default;
                    sanitized_line.push(ch);
                }
                _ => sanitized_line.push(ch),
            },
            State::InKey => {
                if ch == '\"' {
                    state = State::InValue;
                }
            }
            State::InValue => {
                if ch == '\"' {
                    state = State::InTag;
                }
            }
        }

        position += ch.len_utf8();
    }

    sanitized_line
}

fn build_docset_from_map_with_progress<'de, M>(docset_name: &str, mut map: M) -> ResultS
where
    M: MapAccess<'de>,
{
    #[inline]
    #[cfg(target_family = "windows")]
    fn sanitize_filename_for_windows(filename: String) -> String {
        const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
        filename
            .chars()
            .map(|c| if FORBIDDEN_CHARS.contains(&c) { '_' } else { c })
            .collect::<String>()
    }

    let docset_path = get_docset_path(docset_name)?;
    let mut unpacked_amount = 1;

    while let Some((file_path, contents)) = map
        .next_entry::<String, String>()
        .map_err(|err| err.to_string())?
    {
        #[cfg(target_family = "windows")]
        let file_path = sanitize_filename_for_windows(file_path);
        let file_path = PathBuf::from(file_path);

        if let Some(parent) = file_path.parent() {
            create_dir_all(docset_path.join(parent))
                .map_err(|err| format!("Could not create `{}`: {err}", parent.display()))?;
        }

        let mut file_name_html = file_path.as_os_str().to_owned();
        file_name_html.push(".html");

        let file_path = docset_path.join(&file_name_html);

        let file = File::create(&file_path)
            .map_err(|err| format!("Could not create `{}`: {err}", file_path.display()))?;
        let mut writer = BufWriter::new(file);

        let sanitized_contents = sanitize_html_line(contents);

        writer
            .write_all(sanitized_contents.trim().as_bytes())
            .map_err(|err| format!("Could not write to `{}`: {err}", file_path.display()))?;

        print!("Unpacked {unpacked_amount} files...\r");

        unpacked_amount += 1;
    }
    println!();

    Ok(())
}

struct FileVisitor {
    docset_name: String,
}

impl<'de> Visitor<'de> for FileVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string key and a string value")
    }

    fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        build_docset_from_map_with_progress(&self.docset_name, map).map_err(|err| {
            Error::custom(format!(
                "Error while building `{}`: {err}",
                self.docset_name
            ))
        })?;
        Ok(())
    }
}

fn build_docset_from_db_json(docset_name: &String) -> ResultS {
    let docset_path = get_docset_path(docset_name)?;
    let db_json_path = docset_path.join("db").with_extension("json");

    let file = File::open(&db_json_path)
        .map_err(|err| format!("Could not open `{}`: {err}", db_json_path.display()))?;

    let reader = BufReader::new(file);

    let mut db_json_deserializer = serde_json::Deserializer::from_reader(reader);

    let file_visitor = FileVisitor {
        docset_name: docset_name.to_owned(),
    };
    db_json_deserializer
        .deserialize_map(file_visitor)
        .map_err(|err| format!("Could not deserialize `{}`: {err}", db_json_path.display()))?;

    remove_file(&db_json_path).map_err(|err| {
        format!(
            "Could not remove `{}` after building {docset_name}: {err}",
            db_json_path.display()
        )
    })?;

    Ok(())
}

pub(crate) fn download<Args>(mut args: Args) -> ResultS
where
    Args: Iterator<Item = String>,
{
    let mut flag_force;
    let mut flag_help;

    let mut flags = flags![
        flag_force: BoolFlag, ["-f", "--force"],
        flag_help: BoolFlag,  ["--help"]
    ];

    let args = parse_flags(&mut args, &mut flags).map_err(|err| get_flag_error(&err))?;
    if flag_help || args.is_empty() {
        return show_download_help();
    }

    if !is_docs_json_exists()? {
        return Err("The list of available documents has not yet been downloaded. Please run `fetch` first.".to_string());
    }

    let docs = deserialize_docs_json()?;

    let mut successful_downloads = 0;

    for docset in args.iter() {
        // Don't print warnings when using with ls -n
        if docset == "[downloaded]" {
            continue;
        }

        if !flag_force && is_docset_downloaded(docset)? {
            print_warning!(
                "Docset `{docset}` is already downloaded. \
                If you still want to update it, re-run this command with `--force`"
            );
            continue;
        } else if is_docset_in_docs_or_print_warning(docset, &docs) {
            println!("Downloading `{docset}`...");
            download_db_and_index_json_with_progress(docset, &docs)?;

            println!("Extracting to `{}`...", get_docset_path(docset)?.display());
            build_docset_from_db_json(docset)?;

            successful_downloads += 1;
        }
    }

    match successful_downloads {
        0 => {}
        1 => println!("{BOLD}Install has successfully finished{RESET}."),
        _ => println!("{BOLD}{successful_downloads} items were successfully installed{RESET}."),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_html() {
        let html_text = r#"
<summary>
    <section id="method.new" class="method">
        <span class="rightside">
            <a class="srclink" href="https://doc.rust-lang.org/src/alloc/vec/mod.rs.html#420">source</a>
            <span class="since" title="const since 1.39.0">
                const: 1.39.0
            </span>
        </span>
        <pre class="code-header" data-language="rust">
            pub const fn new() -> Vec<T, Global>;
        </pre>
    </section>
</summary>
        "#;

        let should_be = r#"
<summary>
    <section id="method.new" >
        <span >
            <a  href="https://doc.rust-lang.org/src/alloc/vec/mod.rs.html#420">source</a>
            <span  >
                const: 1.39.0
            </span>
        </span>
        <pre  >
            pub const fn new() -> Vec<T, Global>;
        </pre>
    </section>
</summary>
        "#;

        let result = sanitize_html_line(html_text.to_owned());

        assert_eq!(result, should_be);
    }
}
