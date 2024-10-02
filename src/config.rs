use std::fmt;

use colored::Colorize;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub smtp: SmtpConfig,
    pub app: AppSettings,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SmtpConfig {
    pub username: String,
    pub password: String,
    pub server: String,
    pub port: u16,
    pub to: String,
    pub from: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppSettings {
    pub loop_interval_seconds: u64,
    pub rate_limit: usize,
}

// Implementing Display for AppConfig
impl fmt::Display for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}:\n{}\n\n{}:\n{}",
            "SMTP Configuration".blue().bold(),
            self.smtp,
            "Application Settings".green().bold(),
            self.app
        )
    }
}

// Implementing Display for SmtpConfig
impl fmt::Display for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "  {}: {}\n  {}: {}\n  {}: {}\n  {}: {}\n  {}: {}\n  {}: {}",
            "Username".cyan().bold(),
            self.username,
            "Password".red().bold(),
            "********", // Hide actual password
            "Server".cyan().bold(),
            self.server,
            "Port".cyan().bold(),
            self.port,
            "Recipient Email (To)".yellow().bold(),
            self.to,
            "Sender Email (From)".yellow().bold(),
            self.from
        )
    }
}

// Implementing Display for AppSettings
impl fmt::Display for AppSettings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "  {}: {}\n  {}: {}",
            "Loop Interval (seconds)".magenta().bold(),
            self.loop_interval_seconds,
            "Rate Limit".magenta().bold(),
            self.rate_limit
        )
    }
}
