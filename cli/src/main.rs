use log::error;
use std::sync::{Arc, Mutex};
use zecwallet_cli::{
    attempt_recover_seed, configure_clapapp, report_permission_error, start_interactive, startup,
    version::VERSION,
};
use zecwalletlitelib::{
    lightclient::{self, lightclient_config::LightClientConfig},
    MainNetwork, TestNetwork,
};

pub fn main() {
    // Get command line arguments
    use clap::{App, Arg};
    let fresh_app = App::new("Zecwallet CLI");
    let configured_app = configure_clapapp!(fresh_app).arg(
        Arg::with_name("config")
            .long("config")
            .value_name("config")
            .help("Path to config file (default: ~/.zcash/zecwallet.conf)")
            .takes_value(true),
    );
    let matches = configured_app.get_matches();

    if matches.is_present("recover") {
        attempt_recover_seed(matches.value_of("password").map(|s| s.to_string()));
        return;
    }

    // Load config file (CLI args override config file values below)
    let wallet_config = zecwallet_cli::config::load_config(matches.value_of("config"));

    let command = matches.value_of("COMMAND");
    let params = matches
        .values_of("PARAMS")
        .map(|v| v.collect())
        .or(Some(vec![]))
        .unwrap();

    // CLI args win over config file.
    // Note: --server has a default_value, so value_of() is never None;
    // use occurrences_of to detect whether the user actually passed it.
    let maybe_server = if matches.occurrences_of("server") > 0 {
        matches.value_of("server").map(|s| s.to_string())
    } else {
        wallet_config.server.clone()
    };

    let maybe_data_dir = matches
        .value_of("data-dir")
        .map(|s| s.to_string())
        .or_else(|| wallet_config.data_dir.clone());

    let seed = matches.value_of("seed").map(|s| s.to_string());
    let maybe_birthday = matches.value_of("birthday");

    if seed.is_some() && maybe_birthday.is_none() {
        eprintln!("ERROR!");
        eprintln!("Please specify the wallet birthday (eg. '--birthday 600000') to restore from seed.");
        eprintln!("This should be the block height where the wallet was created. If you don't remember the block height, you can pass '--birthday 0' to scan from the start of the blockchain.");
        return;
    }

    let birthday = match maybe_birthday.unwrap_or("0").parse::<u64>() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Couldn't parse birthday. This should be a block number. Error={}", e);
            return;
        }
    };

    let nosync = matches.is_present("nosync");
    let testnet = matches.is_present("testnet") || wallet_config.testnet;

    let is_serve = command == Some("serve");
    let use_rpc = wallet_config.rpcuser.is_some() && wallet_config.rpcpassword.is_some();

    // Handle commands that need no wallet or server connection (e.g. help).
    if let Some(cmd) = command {
        let local_params: Vec<&str> = params.iter().map(|s| *s).collect();
        if let Some(output) = zecwallet_cli::local_command(cmd, &local_params) {
            println!("{}", output);
            return;
        }
    }

    // Non-serve commands with RPC configured: forward to running daemon
    if !is_serve && use_rpc {
        let cmd = command.unwrap_or("help");
        let result = zecwallet_cli::client::call_server(
            &wallet_config.rpcbind,
            wallet_config.rpcport,
            wallet_config.rpcuser.as_deref().unwrap(),
            wallet_config.rpcpassword.as_deref().unwrap(),
            cmd,
            params.iter().map(|s| s.to_string()).collect(),
        );
        match result {
            Ok(s) => println!("{}", s),
            Err(e) => eprintln!("Error: {}", e),
        }
        return;
    }

    // Direct execution path (or serve)
    let server = LightClientConfig::<MainNetwork>::get_server_or_default(maybe_server);

    // Test to make sure the server has all of scheme, host and port
    if server.scheme_str().is_none() || server.host().is_none() || server.port().is_none() {
        eprintln!(
            "Please provide the --server parameter as [scheme]://[host]:[port].\nYou provided: {}",
            server
        );
        return;
    }

    eprintln!("Network: {}", if testnet { "testnet" } else { "mainnet" });

    let startup_chan = if testnet {
        startup(TestNetwork, server, seed, birthday, maybe_data_dir, !nosync, !is_serve, true)
    } else {
        startup(MainNetwork, server, seed, birthday, maybe_data_dir, !nosync, !is_serve, false)
    };

    let (command_tx, resp_rx) = match startup_chan {
        Ok(c) => c,
        Err(e) => {
            let emsg = format!(
                "Error during startup: {}\nNetwork is set to {}. If this doesn't match your wallet, check 'testnet' in your config file (~/.zcash/zecwallet.conf).\nIf you repeatedly run into this issue, you might have to restore your wallet from your seed phrase.",
                e,
                if testnet { "testnet" } else { "mainnet" }
            );
            eprintln!("{}", emsg);
            error!("{}", emsg);
            if cfg!(target_os = "unix") {
                match e.raw_os_error() {
                    Some(13) => report_permission_error(),
                    _ => {}
                }
            };
            return;
        }
    };

    if is_serve {
        let rpcuser = wallet_config.rpcuser.unwrap_or_default();
        let rpcpassword = wallet_config.rpcpassword.unwrap_or_default();
        let channel = Arc::new(Mutex::new((command_tx, resp_rx)));
        zecwallet_cli::server::start_server(
            &wallet_config.rpcbind,
            wallet_config.rpcport,
            rpcuser,
            rpcpassword,
            channel,
            wallet_config.sync_interval,
        );
    } else if command.is_none() {
        start_interactive(command_tx, resp_rx);
    } else {
        command_tx
            .send((
                command.unwrap().to_string(),
                params.iter().map(|s| s.to_string()).collect::<Vec<String>>(),
            ))
            .unwrap();

        match resp_rx.recv() {
            Ok(s) => println!("{}", s),
            Err(e) => {
                let e = format!("Error executing command {}: {}", command.unwrap(), e);
                eprintln!("{}", e);
                error!("{}", e);
            }
        }

        // Save before exit
        command_tx.send(("save".to_string(), vec![])).unwrap();
        resp_rx.recv().unwrap();
    }
}
