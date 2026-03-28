use thiserror::Error;

#[derive(Debug, Error)]
pub enum HuginError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Probe error for '{probe}': {message}")]
    Probe { probe: String, message: String },

    #[error("InfluxDB write error: {0}")]
    Influx(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Secret file error at '{path}': {message}")]
    Secret { path: String, message: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, HuginError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_config() {
        let e = HuginError::Config("missing field".into());
        assert!(e.to_string().contains("Configuration error"));
    }

    #[test]
    fn error_display_probe() {
        let e = HuginError::Probe {
            probe: "web".into(),
            message: "timeout".into(),
        };
        assert!(e.to_string().contains("web"));
        assert!(e.to_string().contains("timeout"));
    }

    #[test]
    fn error_display_secret() {
        let e = HuginError::Secret {
            path: "/run/secrets/token".into(),
            message: "not found".into(),
        };
        assert!(e.to_string().contains("/run/secrets/token"));
    }
}
