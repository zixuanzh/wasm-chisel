extern crate libchisel;
extern crate parity_wasm;
#[macro_use]
extern crate clap;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;

use std::env;
use std::fs::{read, read_to_string};
use std::process;

use libchisel::{checkstartfunc::*, verifyexports::*};

use clap::{App, Arg, ArgMatches, SubCommand};
use libchisel::*;
use parity_wasm::elements::{deserialize_buffer, Module};
use serde_yaml::{from_str, Value};

// Error messages
static ERR_NO_SUBCOMMAND: &'static str = "No subcommand provided.";
static ERR_FAILED_OPEN_CONFIG: &'static str = "Failed to open configuration file.";
static ERR_FAILED_OPEN_BINARY: &'static str = "Failed to open wasm binary.";
static ERR_FAILED_PARSE_CONFIG: &'static str = "Failed to parse configuration file.";
static ERR_CONFIG_INVALID: &'static str = "Config is invalid.";
static ERR_CONFIG_MISSING_FILE: &'static str = "Config missing file path to chisel.";
static ERR_INPUT_FILE_TYPE_MISMATCH: &'static str = "Entry 'file' does not map to a string.";
static ERR_MODULE_TYPE_MISMATCH: &'static str =
    "An entry 'module' does not point to a key-value map.";
static ERR_PRESET_TYPE_MISMATCH: &'static str =
    "A field 'preset' belonging to a module is not a string";
static ERR_DESERIALIZE_MODULE: &'static str = "Failed to deserialize the wasm binary.";

// Other constants
static DEFAULT_CONFIG_PATH: &'static str = "chisel.yml";

/// Chisel configuration structure. Contains a file to chisel and a list of modules configurations.
struct ChiselContext {
    file: String,
    modules: Vec<ModuleContext>,
}

struct ModuleContext {
    module_name: String,
    preset: Option<String>,
}

impl ChiselContext {
    fn from_ruleset(ruleset: &Value) -> Result<Self, &'static str> {
        if let Value::Mapping(rules) = ruleset {
            let mut filepath = String::new();
            let mut module_confs: Vec<ModuleContext> = vec![];
            // If we have more than one ruleset, only use the first valid one.
            // TODO: allow selecting a ruleset
            if let Some((name, mut config)) =
                rules.iter().find(|(left, right)| match (left, right) {
                    (Value::String(_s), Value::Mapping(_m)) => true,
                    _ => false,
                }) {
                // First, look for the 'file' entry.
                if let Some(path) = config
                    .as_mapping()
                    .unwrap()
                    .get(&Value::String(String::from("file")))
                {
                    if let Some(path_raw) = path.as_str() {
                        filepath = path_raw.to_string();
                    } else {
                        return Err(ERR_INPUT_FILE_TYPE_MISMATCH);
                    }
                    println!("Using file: {}", filepath);
                } else {
                    return Err(ERR_CONFIG_MISSING_FILE);
                }

                // Parse all valid module entries. Unwrap is ok here because we
                // established earlier that config is a Mapping.
                let mut config_clone = config.as_mapping().unwrap().clone();
                config_clone.remove(&Value::String(String::from("file"))); // Remove "file" so we don't interpret it as a module.

                let mut config_itr = config_clone.iter();
                // Read modules while there are still modules left.
                while let Some(module) = config_itr.next() {
                    if let (Value::String(mod_name), Value::Mapping(mod_config)) = module {
                        let mod_pre = if let Some(p) =
                            mod_config.get(&Value::String(String::from("preset")))
                        {
                            if !p.is_string() {
                                return Err(ERR_PRESET_TYPE_MISMATCH);
                            }
                            Some(String::from(p.as_str().unwrap()))
                        } else {
                            None
                        };
                        println!(
                            "Loaded module '{}' with preset '{}'",
                            mod_name,
                            mod_pre.clone().unwrap_or("None".to_string())
                        );
                        module_confs.push(ModuleContext::with_fields(mod_name.clone(), mod_pre));
                    } else {
                        return Err(ERR_MODULE_TYPE_MISMATCH);
                    }
                }
            } else {
                return Err(ERR_CONFIG_INVALID);
            }

            Ok(ChiselContext {
                file: filepath,
                modules: module_confs,
            })
        } else {
            Err(ERR_CONFIG_INVALID)
        }
    }

    fn file(&self) -> &String {
        &self.file
    }

    fn get_modules(&self) -> &Vec<ModuleContext> {
        &self.modules
    }
}

impl ModuleContext {
    fn with_fields(module: String, pre: Option<String>) -> Self {
        ModuleContext {
            module_name: module,
            preset: pre,
        }
    }

    fn fields(&self) -> (&String, &Option<String>) {
        (&self.module_name, &self.preset)
    }
}

fn err_exit(msg: &str) -> ! {
    println!("{}: {}", crate_name!(), msg);
    process::exit(-1);
}

fn yaml_configure(yaml: String) -> Result<ChiselContext, &'static str> {
    if let Ok(ruleset) = serde_yaml::from_str::<Value>(yaml.as_str()) {
        ChiselContext::from_ruleset(&ruleset)
    } else {
        Err(ERR_FAILED_PARSE_CONFIG)
    }
}

fn execute_module(context: &ModuleContext, module: &Module) -> bool {
    let (conf_name, conf_preset) = context.fields();
    let preset = conf_preset
        .clone()
        .unwrap_or(String::from("ewasm"))
        .to_string();

    let name = conf_name.as_str();
    match name {
        "verifyexports" => {
            if let Ok(chisel) = VerifyExports::with_preset(&preset) {
                chisel.validate(module).unwrap_or(false)
            } else {
                false
            }
        },
        "verifyimports" => {
            if let Ok(chisel) = VerifyExports::with_preset(&preset) {
                chisel.validate(module).unwrap_or(false)
            } else {
                false
            }
        },
        "checkstartfunc" => {
            //NOTE: checkstartfunc takes a bool for configuration. false by default for now.
            let chisel = CheckStartFunc::new(false);
            let ret = chisel.validate(module).unwrap_or(false);
            ret
        }, /*
        "deployer" => 
        "trimexports"
        "remapimports"
        */
        _ => false,
    }
}

fn chisel_execute(context: &ChiselContext) -> Result<bool, &'static str> {
    if let Ok(buffer) = read(context.file()) {
        if let Ok(module) = deserialize_buffer::<Module>(&buffer) {
            let chisel_results = context
                .get_modules()
                .iter()
                .map(|ctx| execute_module(ctx, &module))
                .fold(true, |b, e| e & b);
            Ok(chisel_results)
        } else {
            Err(ERR_DESERIALIZE_MODULE)
        }
    } else {
        Err(ERR_FAILED_OPEN_BINARY)
    }
}

fn chisel_subcommand_run(args: &ArgMatches) -> i32 {
    let config_path = args.value_of("CONFIG").unwrap_or(DEFAULT_CONFIG_PATH);

    if let Ok(conf) = read_to_string(config_path) {
        match yaml_configure(conf) {
            Ok(ctx) => match chisel_execute(&ctx) {
                Ok(result) => if result {
                    return 0;
                } else {
                    return 1;
                },
                Err(msg) => err_exit(msg),
            },
            Err(msg) => err_exit(msg),
        };
    } else {
        err_exit(ERR_FAILED_OPEN_CONFIG);
    }
}

pub fn main() {
    let cli_matches = App::new("chisel")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .subcommand(
            SubCommand::with_name("run")
                .about("Runs chisel with the closest configuration file.")
                .arg(
                    Arg::with_name("CONFIG")
                        .short("c")
                        .long("config")
                        .help("Sets a custom configuration file")
                        .value_name("CONF_FILE")
                        .takes_value(true),
                ),
        ).get_matches();

    match cli_matches.subcommand() {
        ("run", Some(subcmd_matches)) => process::exit(chisel_subcommand_run(subcmd_matches)),
        _ => err_exit(ERR_NO_SUBCOMMAND),
    };
}
