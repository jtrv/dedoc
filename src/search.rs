#![allow(clippy::useless_format)]

use std::borrow::Cow;
use std::fs::{read_dir, File};
use std::io::{BufRead, BufReader, BufWriter};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use toiletcli::flags;
use toiletcli::flags::*;

use crate::common::ResultS;
use crate::common::{
    deserialize_docs_json, get_docset_path, get_flag_error, get_program_directory,
    get_terminal_width, is_docs_json_exists, is_docset_downloaded,
    is_docset_in_docs_or_print_warning, print_page_from_docset, split_to_item_and_fragment,
};
use crate::common::{
    BOLD, DOC_PAGE_EXTENSION, GRAY, GRAYER, GRAYEST, GREEN, LIGHT_GRAY, PROGRAM_NAME, RESET,
};
use crate::print_warning;

fn show_search_help() -> ResultS {
    println!(
        "\
{GREEN}USAGE{RESET}
    {BOLD}{PROGRAM_NAME} search{RESET} [-wipofc] <docset> <query>
    List docset pages that match your query.

{GREEN}OPTIONS{RESET}
    -w, --whole                     Search for the whole sentence.
    -i, --ignore-case               Ignore character case.
    -p, --precise                   Look inside files (like `grep`).
    -o, --open <number>             Open n-th result.
    -f, --ignore-fragment           For --open: ignore the fragment and open the entire page.
    -c, --columns <number>          For --open: make output N columns wide.
        --help                      Display help message."
    );
    Ok(())
}

#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct ExactResult {
    item: String,
    fragment: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct VagueResult {
    item: String,
    contexts: Vec<String>,
}

// Flags that change search result must be added here for cache to be updated.
#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
struct SearchFlags {
    case_insensitive: bool,
    precise: bool,
    whole: bool,
    ignore_fragment: bool,
}

// Sometimes search results are big, and it's cheaper to check a small file if current search
// options match cached ones, to deserialize the whole search cache.
#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct SearchOptions<'a> {
    query: Cow<'a, str>,
    docset: Cow<'a, str>,
    flags: Cow<'a, SearchFlags>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SearchCache<'a> {
    exact_results: Cow<'a, [ExactResult]>,
    vague_results: Cow<'a, [VagueResult]>,
}

pub(crate) fn try_use_cache<'a>(search_options: &SearchOptions) -> Option<SearchCache<'a>> {
    let program_dir = get_program_directory().ok()?;
    let cache_options_path = program_dir.join("search_cache_options.json");

    {
        let cache_options_file = File::open(cache_options_path).ok()?;
        let cache_options_reader = BufReader::new(cache_options_file);

        let cached_search_options: SearchOptions =
            serde_json::from_reader(cache_options_reader).ok()?;

        if cached_search_options != *search_options {
            return None;
        }
    }

    let cache_path = program_dir.join("search_cache.json");

    let cache_file = File::open(cache_path).ok()?;
    let cache_reader = BufReader::new(cache_file);

    let cache: SearchCache = serde_json::from_reader(cache_reader).ok()?;

    Some(cache)
}

fn cache_search_results(search_options: &SearchOptions, search_cache: &SearchCache) -> ResultS {
    let program_dir = get_program_directory()?;

    {
        let cache_options_path = program_dir.join("search_cache_options.json");
        let cache_options_file = File::create(&cache_options_path).map_err(|err| {
            format!(
                "Could not create cache options at `{}`: {err}",
                cache_options_path.display()
            )
        })?;

        let cache_options_writer = BufWriter::new(cache_options_file);

        serde_json::to_writer(cache_options_writer, &search_options).map_err(|err| {
            format!(
                "Could not write cache options at `{}`: {err}",
                cache_options_path.display()
            )
        })?;
    }

    {
        let cache_path = program_dir.join("search_cache.json");
        let cache_file = File::create(&cache_path).map_err(|err| {
            format!(
                "Could not create cache at `{}`: {err}",
                cache_path.display()
            )
        })?;

        let cache_writer = BufWriter::new(cache_file);

        serde_json::to_writer(cache_writer, &search_cache)
            .map_err(|err| format!("Could not write cache at `{}`: {err}", cache_path.display()))?;
    }

    Ok(())
}

#[allow(dead_code)]
#[derive(Deserialize, Default)]
struct IndexEntry {
    name: String,
    path: String,
    #[serde(skip)]
    r#type: String,
}

#[derive(Deserialize)]
struct IndexJson {
    entries: Vec<IndexEntry>,
}

type ExactMatches = Vec<ExactResult>;
type VagueMatches = Vec<VagueResult>;

fn search_docset_in_filenames(
    docset_name: &str,
    query: &str,
    case_insensitive: bool,
) -> Result<ExactMatches, String> {
    let docset_path = get_docset_path(docset_name)?;
    let index_json_path = docset_path.join("index.json");

    let index_exists = index_json_path.try_exists().map_err(|err| {
        format!(
            "Could not check if `{}` exists: {err}",
            index_json_path.display()
        )
    })?;

    if !index_exists {
        let message = format!("\
Index file does not exist for `{docset_name}`. Docsets that were downloaded prior to version `0.2.0` are incompatible. \
Please redownload the docset with `download {docset_name} --force`."
        );
        return Err(message);
    }

    let file = File::open(&index_json_path)
        .map_err(|err| format!("Could not open `{}`: {err}", index_json_path.display()))?;

    let reader = BufReader::new(file);

    let index: IndexJson = serde_json::from_reader(reader).map_err(|err| {
        format!(
            "Could not deserialize `{}`: {err}",
            index_json_path.display()
        )
    })?;

    let mut items = vec![];

    if case_insensitive {
        let query = query.to_lowercase();

        for entry in index.entries {
            let lowercase_name = entry.name.to_lowercase();
            let lowercase_path = entry.path.to_lowercase();

            if lowercase_name.contains(&query) || lowercase_path.contains(&query) {
                let (item, fragment) = split_to_item_and_fragment(entry.path)?;

                let exact_match = ExactResult { item, fragment };

                items.push(exact_match);
            }
        }
    } else {
        for entry in index.entries {
            if entry.name.contains(query) || entry.path.contains(query) {
                let (item, fragment) = split_to_item_and_fragment(entry.path)?;

                let exact_match = ExactResult { item, fragment };

                items.push(exact_match);
            }
        }
    }

    items.sort_unstable();

    Ok(items)
}

fn get_context_around_query(html_line: &str, index: usize, query_len: usize) -> String {
    const BOUND_OFFSET: usize = (80 - 6 - 8) / 2; // (80 columns - ["...".len() * 2] - [TAB.len() * 2]) / 2 sides

    let lower_bound = index.saturating_sub(BOUND_OFFSET);
    let upper_bound = (index + query_len).saturating_add(BOUND_OFFSET);
    let word_end_index = index + query_len;

    let start_pos = html_line
        .char_indices()
        .rev()
        .find(|&(idx, _)| idx <= lower_bound)
        .map_or(0, |(idx, _)| idx);

    let end_pos = html_line
        .char_indices()
        .skip_while(|&(idx, _)| idx < word_end_index)
        .find(|&(idx, _)| idx >= upper_bound)
        .map_or(html_line.len(), |(idx, _)| idx);

    html_line[start_pos..end_pos].trim().to_owned()
}

// Item is a file path without a file extension which is relative to docset directory
fn convert_path_to_item(path: PathBuf, docset_path: &PathBuf) -> Result<String, String> {
    let item = path
        .strip_prefix(docset_path)
        .map_err(|err| err.to_string())?
        .with_extension("")
        .display()
        .to_string();

    Ok(item)
}

fn search_docset_precisely(
    docset_name: &str,
    query: &str,
    case_insensitive: bool,
) -> Result<(ExactMatches, VagueMatches), String> {
    let docset_path = get_docset_path(docset_name)?;

    let internal_query = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_owned()
    };

    fn visit_dir_with_query(
        original_path: &PathBuf,
        path: &PathBuf,
        query: &String,
        case_insensitive: bool,
    ) -> Result<(ExactMatches, VagueMatches), String> {
        let mut exact_files = vec![];
        let mut vague_results = vec![];

        let dir = read_dir(path)
            .map_err(|err| format!("Could not read `{}` directory: {err}", path.display()))?;

        for entry in dir {
            let entry = entry.map_err(|err| format!("Could not read file: {err}"))?;

            let os_file_name = entry.file_name();

            let file_type = entry
                .file_type()
                .map_err(|err| format!("Could not read file type of {os_file_name:?}: {err}"))?;

            if file_type.is_dir() {
                let (mut exact, mut vague) =
                    visit_dir_with_query(original_path, &entry.path(), query, case_insensitive)?;

                exact_files.append(&mut exact);
                vague_results.append(&mut vague);
            }

            let mut file_name = os_file_name.to_string_lossy().to_string();

            if !file_name.ends_with(DOC_PAGE_EXTENSION) {
                continue;
            }

            if case_insensitive {
                file_name.make_ascii_lowercase();
            }

            let file_path = entry.path();

            if file_name.contains(query) {
                let item = convert_path_to_item(file_path, original_path)?;
                let exact_match = ExactResult {
                    item,
                    fragment: None,
                };
                exact_files.push(exact_match);
            } else {
                let file = File::open(&file_path)
                    .map_err(|err| format!("Could not open `{}`: {err}", file_path.display()))?;

                let query_len = query.len();

                let mut contexts = vec![];

                let mut reader = BufReader::new(file);
                let mut string_buffer = String::new();

                while let Ok(size) = reader.read_line(&mut string_buffer) {
                    if size == 0 {
                        break;
                    }

                    let display_context = if case_insensitive {
                        Cow::Owned(string_buffer.to_lowercase())
                    } else {
                        Cow::Borrowed(&string_buffer)
                    };

                    if let Some(index) = display_context.find(query) {
                        let context = get_context_around_query(&string_buffer, index, query_len);

                        contexts.push(context);
                    }

                    string_buffer.clear();
                }

                if !contexts.is_empty() {
                    let item = convert_path_to_item(file_path, original_path)?;
                    let vague_result = VagueResult { item, contexts };
                    vague_results.push(vague_result);
                }
            }
        }

        Ok((exact_files, vague_results))
    }

    let (mut exact_files, mut vague_results) = visit_dir_with_query(
        &docset_path,
        &docset_path,
        &internal_query,
        case_insensitive,
    )?;

    exact_files.sort_unstable();
    vague_results.sort_unstable();

    let items = (exact_files, vague_results);

    Ok(items)
}

const TAB: &str = "    ";
const HALF_TAB: &str = "  ";

fn print_vague_search_results(search_results: &[VagueResult], mut start_index: usize) -> ResultS {
    for result in search_results {
        println!(
            "{GRAY}{start_index:>4}{RESET}{HALF_TAB}{}{GRAY}",
            result.item
        );

        for context in &result.contexts {
            println!(
                "{TAB}{TAB}{GRAYER}...{RESET}{LIGHT_GRAY}{}{}{RESET}{GRAYER}...{RESET}",
                GRAYEST.bg(),
                context
            );
        }

        start_index += 1;
    }

    Ok(())
}

fn print_search_results(search_results: &[ExactResult], mut start_index: usize) -> ResultS {
    let mut prev_item = "";

    // Group fragments by an item.
    for result in search_results {
        if let Some(fragment) = &result.fragment {
            if result.item == prev_item {
                println!(
                    "{TAB}{HALF_TAB}{GRAYER}{start_index:>4}{HALF_TAB}{GRAY}#{}{RESET}",
                    fragment
                );
            } else {
                println!(
                    "{GRAY}{start_index:>4}{RESET}{HALF_TAB}{}{GRAY}, #{}{RESET}",
                    result.item, fragment
                );
            }
        } else {
            println!("{GRAY}{start_index:>4}{RESET}{HALF_TAB}{}", result.item);
        }

        prev_item = &result.item;
        start_index += 1;
    }

    Ok(())
}

fn search_impl(
    search_options: SearchOptions,
    // Passing this as a String is needed to check if output was not numeric
    // before parsing it as number
    flag_open: String,
    flag_columns: String,
) -> Result<Vec<String>, String> {
    let mut warnings = vec![];

    let SearchOptions {
        ref docset,
        ref flags,
        ref query,
    } = search_options;

    let open_number = flag_open.parse::<usize>().ok();
    let mut width = get_terminal_width();

    let maybe_columns = flag_columns.parse::<usize>().ok();
    if let Some(col_number) = maybe_columns {
        if col_number == 0 {
            width = 999;
        } else if col_number > 10 {
            width = col_number;
        }
    } else if !flag_columns.is_empty() {
        warnings.push("Invalid number of columns.".to_string());
    }

    if open_number.is_none() {
        // This lets you know whether flag messed up your query
        println!("Searching for `{}`...", search_options.query);
    }

    if flags.precise {
        let (exact_results, vague_results) = if let Some(cache) = try_use_cache(&search_options) {
            (cache.exact_results, cache.vague_results)
        } else {
            let (exact, vague) = search_docset_precisely(docset, query, flags.case_insensitive)?;

            let search_cache = SearchCache {
                exact_results: Cow::Borrowed(&exact),
                vague_results: Cow::Borrowed(&vague),
            };

            let _ = cache_search_results(&search_options, &search_cache).map_err(|err| {
                warnings.push(format!("Could not write cache: {err}."));
            });

            (exact.into(), vague.into())
        };

        let exact_results_offset = exact_results.len();

        if !flag_open.is_empty() {
            match open_number {
                Some(n) if n < 1 || n > exact_results_offset + vague_results.len() => {
                    warnings.push(format!("`--open {n}` is out of bounds."));
                }
                Some(n) if n <= exact_results_offset => {
                    let result = &exact_results[n - 1];
                    let fragment = if flags.ignore_fragment {
                        None
                    } else {
                        result.fragment.as_ref()
                    };
                    print_page_from_docset(docset, &result.item, fragment, width)?;
                    return Ok(warnings);
                }
                Some(n) => {
                    let result = &vague_results[n - exact_results_offset - 1];
                    print_page_from_docset(docset, &result.item, None, width)?;
                    return Ok(warnings);
                }
                _ => {
                    warnings.push(format!("`--open` requires a number."));
                }
            }
        }

        if !exact_results.is_empty() {
            println!("{BOLD}Exact matches in `{docset}`{RESET}:");
            print_search_results(&exact_results, 1)?;
        } else {
            println!("{BOLD}No exact matches in `{docset}`{RESET}.");
        }

        if !vague_results.is_empty() {
            println!("{BOLD}Mentions in other files from `{docset}`{RESET}:");
            print_vague_search_results(&vague_results, exact_results_offset + 1)?;
        } else {
            println!("{BOLD}No mentions in other files from `{docset}`{RESET}.");
        }

        Ok(warnings)
    } else {
        let results = if let Some(cache) = try_use_cache(&search_options) {
            cache.exact_results
        } else {
            let exact = search_docset_in_filenames(docset, query, flags.case_insensitive)?;

            let search_cache = SearchCache {
                exact_results: Cow::Borrowed(&exact),
                vague_results: Cow::Owned(vec![]),
            };

            let _ = cache_search_results(&search_options, &search_cache).map_err(|err| {
                warnings.push(format!("Could not write cache: {err}."));
            });

            exact.into()
        };

        if !flag_open.is_empty() {
            match open_number {
                Some(n) if n < 1 || n > results.len() => {
                    warnings.push(format!("`--open {n}` is out of bounds."));
                }
                Some(n) => {
                    let result = &results[n - 1];
                    let fragment = if flags.ignore_fragment {
                        None
                    } else {
                        result.fragment.as_ref()
                    };
                    print_page_from_docset(docset, &result.item, fragment, width)?;
                    return Ok(warnings);
                }
                _ => {
                    warnings.push(format!("`--open` requires a number."));
                }
            }
        }

        if !results.is_empty() {
            println!("{BOLD}Exact matches in `{docset}`{RESET}:");
            print_search_results(&results, 1)?;
        } else {
            println!("{BOLD}No exact matches in `{docset}`{RESET}.");
        }

        Ok(warnings)
    }
}

pub(crate) fn search<Args>(mut args: Args) -> ResultS
where
    Args: Iterator<Item = String>,
{
    let mut flag_whole;
    let mut flag_columns;
    let mut flag_precise;
    let mut flag_open;
    let mut flag_case_insensitive;
    let mut flag_ignore_fragment;
    let mut flag_help;

    let mut flags = flags![
        flag_columns: StringFlag,        ["-c", "--columns"],
        flag_whole: BoolFlag,            ["-w", "--whole"],
        flag_precise: BoolFlag,          ["-p", "--precise"],
        flag_open: StringFlag,           ["-o", "--open"],
        flag_case_insensitive: BoolFlag, ["-i", "--ignore-case"],
        flag_ignore_fragment: BoolFlag,  ["-f", "--ignore-fragment"],
        flag_help: BoolFlag,             ["--help"]
    ];

    let args = parse_flags(&mut args, &mut flags).map_err(|err| get_flag_error(&err))?;

    if flag_help {
        return show_search_help();
    }

    if !is_docs_json_exists()? {
        return Err("\
The list of available documents has not yet been downloaded. Please run `fetch` first."
            .to_string());
    }

    let mut args = args.into_iter();

    let docset = if let Some(docset_name) = args.next() {
        docset_name
    } else {
        return show_search_help();
    };

    let docs = deserialize_docs_json()?;

    if !is_docset_downloaded(&docset)? {
        if is_docset_in_docs_or_print_warning(&docset, &docs) {
            print_warning!("Docset `{docset}` is not downloaded. Try running `download {docset}`.");
        }
        return Ok(());
    }

    let query = {
        let mut merged_args = args.collect::<Vec<String>>().join(" ");

        if flag_whole {
            merged_args.insert(0, ' ');
            merged_args.push(' ');
            merged_args
        } else {
            merged_args
        }
    };

    let search_flags = SearchFlags {
        precise: flag_precise,
        case_insensitive: flag_case_insensitive,
        whole: flag_whole,
        ignore_fragment: flag_ignore_fragment,
    };

    let search_options = SearchOptions {
        query: Cow::Borrowed(&query),
        docset: Cow::Borrowed(&docset),
        flags: Cow::Borrowed(&search_flags),
    };

    // Print warnings only after search results
    let warnings = search_impl(search_options, flag_open, flag_columns)?;
    for warning in warnings {
        print_warning!("{}", warning);
    }

    Ok(())
}
