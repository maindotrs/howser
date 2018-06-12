#[macro_use]
extern crate clap;
extern crate doogie;
extern crate env_logger;
extern crate howser;
extern crate toml;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use doogie::parse_document;
use howser::document::Document;
use howser::errors::{HowserError, HowserResult, ValidationProblem};
use howser::reporters::{make_cli_report, CLIOption};
use howser::validator::Validator;
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::str;
use std::collections::BTreeMap;
use toml::Value;

fn main() {
    env_logger::init();

    let app = make_app();
    let matches = app.get_matches();

    if let Err(e) = run(&matches) {
        println!("{}", e.description());
        let mut inner_err = e.cause();
        while let Some(error) = inner_err {
            println!("{}", error.description());
            inner_err = error.cause();
        }
        std::process::exit(1);
    } else {
        std::process::exit(0);
    }
}

fn run(args: &ArgMatches) -> HowserResult<()> {
    let (issues, options) = match args.subcommand() {
        ("check", Some(sub_m)) => {
            let options = vec![CLIOption::VerboseMode(sub_m.is_present("verbose"))];
            let filename = args
                .value_of("prescription")
                .ok_or(HowserError::RuntimeError(
                    "Error parsing prescription filename.".to_string()))?;

            (check(filename)?, options)
        }
        ("validate", Some(sub_m)) => {
            let options = vec![CLIOption::VerboseMode(sub_m.is_present("verbose"))];
            if sub_m.is_present("pharmacy") {
                let fail_early = sub_m.is_present("fail-early");
                let filename = args
                    .value_of("pharmacy").
                    ok_or(HowserError::RuntimeError(
                        "Pharmacy filename could not be parsed from the argument string.".to_string()))?;
                let pharmacy_contents = get_file_contents(filename)?;
                let specs = &pharmacy_contents.parse::<Value>()?["Specs"];
                let prescription_pairs = specs
                    .as_table()
                    .ok_or(HowserError::RuntimeError(
                        format!("Error parsing pharmacy file {}.", filename)))?;

                (process_pharmacy_file(prescription_pairs, fail_early)?, options)
            } else {
                let rx_name = sub_m
                    .value_of("prescription")
                    .ok_or(HowserError::RuntimeError(
                        "Unable to parse the name of the prescription file.".to_string()))?;
                let document_name = sub_m
                    .value_of("document")
                    .ok_or(HowserError::RuntimeError(
                        "Unable to parse the name of the document file.".to_string()))?;

                (validate(rx_name, document_name)?, options)
            }
        }
        _ => return Err(HowserError::Usage(args.usage().to_string()))
    };
    let cli_report = make_cli_report(&issues, &options);

    println!("{}", cli_report);

    Ok(())
}

fn make_app<'a, 'b>() -> App<'a, 'b> {
    App::new("Howser")
        .about("Document conformity validator for the Rx spec.")
        .version(crate_version!())
        .version_message("Prints version information.")
        .help_message("Prints help information.")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::VersionlessSubcommands)
        .setting(AppSettings::DisableHelpSubcommand)
        .subcommand(
            SubCommand::with_name("check")
                .about("Verifies that an .rx file conforms to the Rx spec.")
                .help_message("Prints help information.")
                .setting(AppSettings::ArgRequiredElseHelp)
                .arg(
                    Arg::with_name("verbose")
                        .short("v")
                        .long("verbose")
                        .help("Use verbose (multiline) output for errors and warnings."),
                )
                .arg(
                    Arg::with_name("prescription")
                        .required(true)
                        .help("Prescription file to check")
                        .takes_value(true)
                        .value_name("PRESCRIPTION"),
                ),
        )
        .subcommand(
            SubCommand::with_name("validate")
                .about("Validates a Markdown document against an .rx Prescription file.")
                .help_message("Prints help information.")
                .setting(AppSettings::ArgRequiredElseHelp)
                .arg(
                    Arg::with_name("pharmacy")
                        .short("-p")
                        .long("pharmacy")
                        .help("Performs validation based on a pharmacy .toml file.")
                        .value_name("PHARMACY")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("fail-early")
                        .short("-e")
                        .long("fail-early")
                        .help("For use with the --pharmacy option. Causes Howser to exit after the first error.")
                )
                .arg(
                    Arg::with_name("verbose")
                        .short("v")
                        .long("verbose")
                        .help("Use verbose (multiline) output for errors and warnings."),
                )
                .arg(
                    Arg::with_name("prescription")
                        .required_unless("pharmacy")
                        .takes_value(true)
                        .value_name("PRESCRIPTION"),
                )
                .arg(
                    Arg::with_name("document")
                        .required_unless("pharmacy")
                        .takes_value(true)
                        .value_name("DOCUMENT"),
                ),
        )
}

fn validate(rx_name: &str, document_name: &str) -> HowserResult<Vec<ValidationProblem>> {
    let rx_root = parse_document(&get_file_contents(rx_name)?);
    let doc_root = parse_document(&get_file_contents(document_name)?);
    let rx = Document::new(&rx_root, Some(rx_name.to_string()))?.into_prescription()?;
    let document = Document::new(&doc_root, Some(document_name.to_string()))?;

    Validator::new(rx, document).validate()
}

fn check(filename: &str) -> HowserResult<Vec<ValidationProblem>> {
    let rx_root = parse_document(&get_file_contents(filename)?);
    let document = Document::new(&rx_root, Some(filename.to_string()))?;

    match document.into_prescription() {
        Err(HowserError::PrescriptionError(warning)) => Ok(vec![Box::new(warning)]),
        Err(error) => Err(error),
        Ok(_) => Ok(Vec::new()),
    }
}

fn process_pharmacy_file(spec_pairs: &BTreeMap<String, Value>, fail_early: bool) -> HowserResult<Vec<ValidationProblem>> {
    let mut report: Vec<ValidationProblem> = Vec::new();

    for (rx_file, doc_value) in spec_pairs {
        let doc_file = doc_value
                .as_str()
                .ok_or(HowserError::RuntimeError(
                    "The document corresponding to {} could not be parsed as a string.".to_string()))?;

        let mut problems = validate(rx_file, doc_file)?;
        if fail_early && !problems.is_empty() {
            return Ok(problems);
        } else {
            report.append(&mut problems);
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::clap::ErrorKind;

    #[test]
    fn test_validate_subcommand() {
        let app = super::make_app();
        let matches =
            app.get_matches_from(vec!["howser", "validate", "some_template", "some_document"]);
        assert_eq!(matches.subcommand_name(), Some("validate"));
    }

    #[test]
    fn test_validate_subcommand_has_prescription() {
        let app = super::make_app();
        let matches =
            app.get_matches_from(vec!["howser", "validate", "some_template", "some_document"]);
        let sub_m = matches.subcommand_matches("validate").unwrap();
        assert_eq!(sub_m.value_of("prescription").unwrap(), "some_template");
    }

    #[test]
    fn test_validate_subcommand_has_document() {
        let app = super::make_app();
        let matches =
            app.get_matches_from(vec!["howser", "validate", "some_template", "some_document"]);
        let sub_m = matches.subcommand_matches("validate").unwrap();
        assert_eq!(sub_m.value_of("document").unwrap(), "some_document");
    }

    #[test]
    fn test_validate_subcommand_requires_enough_args() {
        let app = super::make_app();
        if let Err(e) = app.get_matches_from_safe(vec!["howser", "validate", "some_template"]) {
            assert_eq!(e.kind, ErrorKind::MissingRequiredArgument);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_validate_subcommand_prevents_extra_args() {
        let app = super::make_app();
        if let Err(e) = app.get_matches_from_safe(vec![
            "howser",
            "validate",
            "some_template",
            "some_document",
            "something_extra",
        ]) {
            assert_eq!(e.kind, ErrorKind::UnknownArgument);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_check_subcommand() {
        let app = super::make_app();
        let matches = app.get_matches_from(vec!["howser", "check", "some_template"]);
        assert_eq!(matches.subcommand_name(), Some("check"));
    }

    #[test]
    fn test_check_subcommand_has_prescription() {
        let app = super::make_app();
        let matches = app.get_matches_from(vec!["howser", "check", "some_template"]);
        let sub_m = matches.subcommand_matches("check").unwrap();
        assert_eq!(sub_m.value_of("prescription").unwrap(), "some_template");
    }

    #[test]
    fn test_check_subcommand_requires_enough_args() {
        let app = super::make_app();
        if let Err(e) = app.get_matches_from_safe(vec!["howser", "check"]) {
            assert_eq!(e.kind, ErrorKind::MissingArgumentOrSubcommand);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_check_subcommand_prevents_extra_args() {
        let app = super::make_app();
        if let Err(e) =
            app.get_matches_from_safe(vec!["howser", "check", "some_template", "some_document"])
        {
            assert_eq!(e.kind, ErrorKind::UnknownArgument);
        } else {
            panic!();
        }
    }

}

/// Returns the textual content of the indicated file
///
/// # Arguments
/// 'file_name': The name of the file to get.
fn get_file_contents(file_name: &str) -> HowserResult<String> {
    let mut file = File::open(file_name)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}