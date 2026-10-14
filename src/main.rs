mod client;
mod config;
mod identity;
mod ports;
mod tui;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "netgear-poe", version, about)]
struct Cli {
    /// Credentials file (default: ~/.netgear-poe.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Machine-readable output, including errors.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive SSH-friendly port panel. Space immediately power-cycles selection.
    Tui,
    /// Switch model, firmware and hardware details.
    Info,
    /// All ports with PoE, link and discovered devices.
    Ports,
    /// Detailed data for one port.
    Port { port: u64 },
    /// All LLDP/ARP discovered devices, optionally filtered by port.
    Devices {
        #[arg(long)]
        port: Option<u64>,
    },
    /// Native PoE reset. Explicit ports and --yes required; no implicit all.
    Cycle {
        #[arg(required = true, num_args = 1..)]
        ports: Vec<u64>,
        /// Authorize disruptive PoE resets on the selected ports.
        #[arg(long)]
        yes: bool,
        /// Validate and print requests without sending resets.
        #[arg(long)]
        dry_run: bool,
    },
}

fn config_path(cli: &Cli) -> Result<PathBuf> {
    cli.config
        .clone()
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".netgear-poe.toml")))
        .context("HOME unset; pass --config")
}

fn run(cli: &Cli) -> Result<Value> {
    if let Command::Cycle { yes, dry_run, .. } = &cli.command {
        ensure!(
            *yes || *dry_run,
            "PoE reset interrupts connected devices; pass --yes or use --dry-run"
        );
    }
    let path = config_path(cli)?;
    let config = config::Config::load(&path)?;
    let switch = client::Switch::login(&config)?;
    match &cli.command {
        Command::Tui => anyhow::bail!("TUI must run interactively"),
        Command::Info => Ok(switch.get("device_info", &[])?["deviceInfo"].clone()),
        Command::Devices { port } => {
            let mut data = switch.neighbors()?;
            if let Some(number) = port {
                data.retain(|v| v["portNum"].as_u64() == Some(*number));
            }
            Ok(json!(data))
        }
        Command::Ports => Ok(json!(ports::snapshot(&switch, &config)?)),
        Command::Port { port } => {
            let data = ports::snapshot(&switch, &config)?;
            let matching: Vec<_> = data.iter().filter(|p| p.port == *port).collect();
            ensure!(matching.len() == 1, "port {port} absent or ambiguous");
            Ok(json!(matching[0]))
        }
        Command::Cycle {
            ports: selected,
            dry_run,
            ..
        } => {
            let mut selected = selected.clone();
            selected.sort_unstable();
            selected.dedup();
            let data = ports::snapshot(&switch, &config)?;
            ports::validate_cycle(&data, &selected)?;
            if *dry_run {
                return Ok(
                    json!({"dry_run":true,"ports":selected,"endpoint":"/api/v1/swcfg_poe_reset","requests":selected.iter().map(|p| client::reset_payload(&[*p])).collect::<Vec<_>>()}),
                );
            }
            // Validate every selection before the first mutation. Send one port per request.
            let mut accepted = Vec::new();
            for port in &selected {
                switch.reset(&[*port]).with_context(|| format!("port {port} reset failed; previously accepted ports: {accepted:?}; stopping"))?;
                accepted.push(*port);
            }
            Ok(
                json!({"ports":accepted,"status":"reset_accepted","note":"Switch accepted reset requests; device recovery is not verified."}),
            )
        }
    }
}

fn display(value: &Value, command: &Command) {
    if matches!(command, Command::Ports) {
        println!("PORT LINK SPEED          POE    WATTS DEVICES PROTECTED");
        for port in value.as_array().unwrap() {
            let link = match port["link_up"].as_bool() {
                Some(true) => "up",
                Some(false) => "down",
                None => "?",
            };
            let poe = if port["poe_capable"] == false {
                "n/a"
            } else if port["poe_enabled"] == true {
                "on"
            } else {
                "off"
            };
            let watts = port["power_watts"]
                .as_f64()
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "-".into());
            println!(
                "{:>4} {:4} {:14} {:4} {:>7} {:>7} {}",
                port["port"],
                link,
                port["speed"].as_str().unwrap_or(""),
                poe,
                watts,
                port["devices"].as_array().map_or(0, Vec::len),
                port["protected"]
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    }
}

fn main() {
    let cli = Cli::parse();
    let result = if matches!(cli.command, Command::Tui) {
        (|| {
            ensure!(!cli.json, "tui cannot be combined with --json");
            tui::run(config_path(&cli)?)?;
            Ok(Value::Null)
        })()
    } else {
        run(&cli)
    };
    match result {
        Ok(data) => {
            if matches!(cli.command, Command::Tui) {
                // Alternate screen restored; leave the SSH terminal clean.
            } else if cli.json {
                println!("{}", json!({"schema_version":1,"ok":true,"data":data}));
            } else {
                display(&data, &cli.command);
            }
        }
        Err(error) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"schema_version":1,"ok":false,"error":format!("{error:#}")})
                );
            } else {
                eprintln!("Error: {error:#}");
            }
            std::process::exit(1);
        }
    }
}
