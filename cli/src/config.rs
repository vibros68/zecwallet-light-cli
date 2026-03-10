use std::fs;
use std::path::PathBuf;

pub struct WalletConfig {
    pub server: Option<String>,
    pub testnet: bool,
    pub data_dir: Option<String>,
    pub rpcbind: String,
    pub rpcport: u16,
    pub rpcuser: Option<String>,
    pub rpcpassword: Option<String>,
    pub sync_interval: u64,
}

impl Default for WalletConfig {
    fn default() -> Self {
        WalletConfig {
            server: None,
            testnet: false,
            data_dir: None,
            rpcbind: "127.0.0.1".to_string(),
            rpcport: 9068,
            rpcuser: None,
            rpcpassword: None,
            sync_interval: 60,
        }
    }
}

pub fn default_config_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".zcash");
    path.push("zecwallet.conf");
    path
}

pub fn load_config(path: Option<&str>) -> WalletConfig {
    let config_path = path.map(PathBuf::from).unwrap_or_else(default_config_path);

    let mut config = WalletConfig::default();

    let content = match fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(_) => return config,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(idx) = line.find('=') {
            let key = line[..idx].trim();
            let value = line[idx + 1..].trim();
            match key {
                "server" => config.server = Some(value.to_string()),
                "testnet" => config.testnet = value == "1" || value.eq_ignore_ascii_case("true"),
                "data_dir" | "datadir" => config.data_dir = Some(value.to_string()),
                "rpcbind" => config.rpcbind = value.to_string(),
                "rpcport" => {
                    if let Ok(p) = value.parse::<u16>() {
                        config.rpcport = p;
                    }
                }
                "rpcuser" => config.rpcuser = Some(value.to_string()),
                "rpcpassword" => config.rpcpassword = Some(value.to_string()),
                "sync_interval" => {
                    if let Ok(s) = value.parse::<u64>() {
                        config.sync_interval = s;
                    }
                }
                _ => {}
            }
        }
    }

    config
}
