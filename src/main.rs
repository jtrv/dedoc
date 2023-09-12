use std::process::ExitCode;
use std::fs::remove_dir_all;

extern crate toiletcli;

use toiletcli::common::name_from_path;
use toiletcli::flags::*;
use toiletcli::flags;

mod docs;
use docs::{
    deserealize_docs_json, download_docset_tar_gz, extract_docset_tar_gz, fetch_docs_json,
    print_html_file, print_page_from_docset, search_docset_in_filenames, search_docset_thoroughly,
    serialize_and_overwrite_docs_json,
};

mod common;
use common::{
    is_docs_json_exists, is_docs_json_old, is_docset_downloaded, is_docset_in_docs,
    print_search_results, get_local_docsets, get_docset_path
};
use common::{BOLD, UNDERLINE, DEFAULT_DOCS_LINK, GREEN, PROGRAM_NAME, RED, RESET, VERSION, YELLOW};

fn show_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME}{RESET} <subcommand> [args]
    Search DevDocs pages from terminal.

{GREEN}SUBCOMMANDS{RESET}
    {BOLD}fetch{RESET}              Fetch available docsets.
    {BOLD}list{RESET}               Show available docsets.
    {BOLD}download{RESET}           Download docsets.
    {BOLD}remove{RESET}             Delete docsets.
    {BOLD}search{RESET}             List pages that match your query.
    {BOLD}open{RESET}               Display specified pages.

{GREEN}OPTIONS{RESET}
    --help                 Display help message. Can be used with subcommands.
    --version, -v          Display version.

The design is not final, and may be subject to change."
);
    println!("{}", help);
    Ok(())
}

fn show_version() -> Result<(), String> {
    let message = format!(
        "\
dedoc {VERSION}
(c) toiletbril <{UNDERLINE}https://github.com/toiletbril{RESET}>

Licensed under GPLv3.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law."
    );
    println!("{}", message);
    Ok(())
}

fn show_search_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} search{RESET} [-ipo] <docset> <query>
    List docset pages that match your query.

{GREEN}OPTIONS{RESET}
    --ignore-case, -i      Ignore character case.
    --precise,     -p      Search more thoroughly and look for mentions in other files.
    --open,        -o <n>  Open n-th exact match.
    --help                 Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn show_open_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} open{RESET} [-i] <docset> <page>
    Print a page. Pages can be searched using `search`.

{GREEN}OPTIONS{RESET}
    --help               Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn show_fetch_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} fetch{RESET} [-f]
    Fetch latest `docs.json` which lists available languages and frameworks.

{GREEN}OPTIONS{RESET}
    --force, -f    Update even if `docs.json` is recent.
    --help         Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn show_list_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} list{RESET} [-la]
    Show available docsets.

{GREEN}OPTIONS{RESET}
    --local, -l    Only show local docsets.
    --all,   -a    Show all version-specific docsets.
    --help         Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn show_download_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} download{RESET} [-f] <docset1> [docset2, ..]
    Download a docset. Available docsets can be displayed using `list`.

{GREEN}OPTIONS{RESET}
    --force, -f    Overwrite downloaded docsets.
    --help         Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn show_remove_help() -> Result<(), String> {
    let help = format!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} remove{RESET} <docset1> [docset2, ..]
    Delete a docset. Only docsets downloaded by {PROGRAM_NAME} can be removed.

{GREEN}OPTIONS{RESET}
    --help         Display help message."
    );
    println!("{}", help);
    Ok(())
}

fn entry<Args>(mut args: &mut Args) -> Result<(), String>
where
    Args: Iterator<Item = String>,
{
    debug!(VERSION);

    let mut flag_version;
    let mut flag_help;

    let mut flags = flags![
        flag_help: BoolFlag,    ["--help"],
        flag_version: BoolFlag, ["--version", "-v"]
    ];

    let subcommand = parse_flags_until_subcommand(&mut args, &mut flags);
    if flag_help { return show_help(); }
    if flag_version { return show_version(); }

    let subcommand = subcommand
        .map_err(|err| format!("{err}. Try `--help` for more information"))?
        .to_lowercase();

    match subcommand.as_str() {
        "f" | "fetch" => {
            let mut flag_help;
            let mut flag_force;

            let mut flags = flags![
                flag_help: BoolFlag,  ["--help"],
                flag_force: BoolFlag, ["--force", "-f"]
            ];

            parse_flags(&mut args, &mut flags)?;
            if flag_help { return show_fetch_help(); }

            if !flag_force && is_docs_json_exists()? && !is_docs_json_old()? {
                let message = format!(
                    "\
{YELLOW}WARNING{RESET}: It seems that your `docs.json` was updated less than a week ago.
{YELLOW}WARNING{RESET}: If you still want to update it, re-run this command with `--force`"
                );
                println!("{}", message);
                return Ok(());
            }

            println!("Fetching `{DEFAULT_DOCS_LINK}`...");
            let docs = fetch_docs_json()?;

            println!("Writing `docs.json`...");
            serialize_and_overwrite_docs_json(docs)?;

            println!("Successfully updated `docs.json`.");
        }
        "l" | "ls" | "list" => {
            let mut flag_help;
            let mut flag_all;
            let mut flag_local;

            let mut flags = flags![
                flag_help: BoolFlag,  ["--help"],
                flag_all: BoolFlag,   ["--all", "-a"],
                flag_local: BoolFlag, ["--local", "-l"]
            ];

            parse_flags(&mut args, &mut flags)?;
            if flag_help {
                return show_list_help();
            }

            let docs_names = if !flag_local {
                let docs = deserealize_docs_json()?;
                docs
                    .iter()
                    .map(|entry| entry.slug.to_string())
                    .collect()
            } else {
                get_local_docsets()?
            };

            let mut docs_names_peekable = docs_names.iter().peekable();

            while let Some(entry) = docs_names_peekable.next() {
                // slug has ~ if it's version-specific
                if !flag_local && !flag_all && entry.find("~").is_some() {
                    continue;
                }

                if is_docset_downloaded(&entry)? {
                    print!("{GREEN}{} [downloaded]{RESET}", entry);
                } else {
                    print!("{}", entry);
                }

                if docs_names_peekable.peek().is_some() {
                    print!(", ");
                } else {
                    println!();
                }
            }
        }
        "d" | "dl" | "download" => {
            let mut flag_help;
            let mut flag_force;

            let mut flags = flags![
                flag_help: BoolFlag,  ["--help"],
                flag_force: BoolFlag, ["--force", "-f"]
            ];

            let args = parse_flags(&mut args, &mut flags)?;
            if flag_help { return show_download_help(); }

            if args.is_empty() {
                return Err("No arguments were provided. Try `download --help` for more information".to_string());
            }

            if !is_docs_json_exists()? {
                return Err("`docs.json` does not exist. Please run `fetch` first".to_string());
            }

            let docs = deserealize_docs_json()?;
            let mut args_iter = args.iter();
            let mut success = 0;

            while let Some(docset) = args_iter.next() {
                if !flag_force && is_docset_downloaded(docset)? {
                    let message = format!("\
{YELLOW}WARNING{RESET}: `{docset}` is already downloaded. If you still want to update it, re-run this command with `--force`");
                    println!("{}", message);
                    continue;
                } else {
                    if !is_docset_in_docs(docset, &docs) {
                        let message = format!(
                            "\
{YELLOW}WARNING{RESET}: Unknown docset `{docset}`. Did you run `fetch`?"
                        );
                        println!("{}", message);
                        continue;
                    }

                    println!("Downloading `{docset}`...");
                    download_docset_tar_gz(docset, &docs)?;

                    println!("Extracting `{docset}` to `{}`...", get_docset_path(docset)?.display());
                    extract_docset_tar_gz(docset)?;

                    success += 1;
                }
            }

            if success > 1 {
                println!("{BOLD}{} items were successfully installed{RESET}.", success);
            } else {
                println!("{BOLD}Install successfully finished{RESET}.");
            }
        }
        "rm" | "remove" => {
            let mut flag_help;

            let mut flags = flags![
                flag_help: BoolFlag, ["--help"]
            ];

            let args = parse_flags(&mut args, &mut flags)?;
            if flag_help { return show_remove_help(); }

            for docset in args.iter() {
                let is_disallowed = {
                    #[cfg(target_family = "windows")]
                    { docset.find("\\").is_some() || docset.find("/").is_some() }

                    #[cfg(target_family = "unix")]
                    { docset.find("/").is_some() }
                };

                if is_disallowed {
                    println!("{YELLOW}WARNING{RESET}: `{docset}` contains forbidden characters.");
                    continue;
                }

                if is_docset_downloaded(docset)? {
                    let docset_path = get_docset_path(docset)?;
                    if docset_path.exists() {
                        println!("Removing `{docset}` from `{}`...", docset_path.display());
                        remove_dir_all(&docset_path)
                            .map_err(|err| format!("Unable to remove {docset_path:?}: {err}"))?;
                    }
                } else {
                    println!("{YELLOW}WARNING{RESET}: `{docset}` is not installed.");
                }
            }
        }
        "s" | "ss" | "search" => {
            let mut flag_help;
            let mut flag_precise;
            let mut flag_open;
            let mut flag_case_insensitive;

            let mut flags = flags![
                flag_help: BoolFlag,             ["--help"],
                flag_precise: BoolFlag,          ["--precise", "-p"],
                flag_open: StringFlag,           ["--open", "-o"],
                flag_case_insensitive: BoolFlag, ["--ignore-case", "-i"]
            ];

            let args = parse_flags(&mut args, &mut flags)?;
            if flag_help { return show_search_help(); }

            let mut args = args.iter();

            let docset = if let Some(_docset) = args.next() {
                _docset
            } else {
                return Err("No docset was provided. Try `search --help` for more information".to_string());
            };

            if !is_docset_downloaded(docset)? {
                let message = format!("`{docset}` docset is not downloaded. Try using `download`");
                return Err(message);
            }

            let mut query = args.fold(String::new(), |base, next| base + next + " ");
            query.pop(); // remove last space

            if flag_precise {
                let (exact, vague) =
                    search_docset_thoroughly(&docset, &query, flag_case_insensitive)?;

                if !flag_open.is_empty() {
                    let n = flag_open.parse::<usize>()
                        .map_err(|err| format!("Unable to parse --open value as number: {err}"))?;

                    if n <= exact.len() && n > 0 {
                        print_html_file(&exact[n - 1])?;
                        return Ok(());
                    } else {
                        println!("{YELLOW}WARNING{RESET}: --open {n} is larger than search result.");
                    }
                }

                if !exact.is_empty() {
                    println!("{BOLD}Exact matches in `{docset}`{RESET}:");
                    print_search_results(exact, &docset)?;
                } else {
                    println!("{BOLD}No exact matches in `{docset}`{RESET}.");
                }

                if !vague.is_empty() {
                    println!("{BOLD}Mentions in other files from `{docset}`{RESET}:");
                    print_search_results(vague, &docset)?;
                } else {
                    println!("{BOLD}No mentions in other files from `{docset}`{RESET}.");
                }
            } else {
                let result = search_docset_in_filenames(&docset, &query, flag_case_insensitive)?;

                if !flag_open.is_empty() {
                    let n = flag_open.parse::<usize>()
                        .map_err(|err| format!("Unable to parse --open value as number: {err}"))?;

                    if n <= result.len() && n > 0 {
                        print_html_file(&result[n - 1])?;
                        return Ok(());
                    } else {
                        println!("{YELLOW}WARNING{RESET}: --open {n} is invalid.");
                    }
                }

                if !result.is_empty() {
                    println!("{BOLD}Exact matches in `{docset}`{RESET}:");
                    print_search_results(result, &docset)?;
                } else {
                    println!("{BOLD}No exact matches in `{docset}`{RESET}.");
                }
            };
        }
        "o" | "open" => {
            let mut flag_help;

            let mut flags = flags![
                flag_help: BoolFlag, ["--help"]
            ];

            let args = parse_flags(&mut args, &mut flags)?;
            if flag_help { return show_open_help(); }

            let mut args = args.iter();

            let docset = if let Some(_docset) = args.next() {
                _docset
            } else {
                return Err("No docset was provided. Try `open --help` for more information.".to_string());
            };

            if !is_docset_downloaded(docset)? {
                let message = format!("`{docset}` docset is not downloaded. Try using `download`");
                return Err(message);
            }

            let mut query = args.fold(String::new(), |base, next| base + next + " ");
            query.pop(); // remove last space

            if query.is_empty() {
                return Err("No page specified. Try `open --help` for more information.".to_string());
            }

            print_page_from_docset(docset, &query)?;
        }
        other => return Err(format!("Unknown subcommand `{other}`")),
    }

    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args();
    let _program_name = name_from_path(&args.next().expect("Progran path is provided"));

    match entry(&mut args) {
        Err(err) => {
            eprintln!("{RED}ERROR{RESET}: {err}");
            ExitCode::FAILURE
        }
        _ => ExitCode::SUCCESS,
    }
}
