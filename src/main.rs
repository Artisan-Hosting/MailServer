use ::config::{Config, File};
use artisan_middleware::communication_proto::receive_message_tcp;
use artisan_middleware::notifications::Email;
use config::AppConfig;
use dusa_collection_utils::errors::{ErrorArrayItem, Errors};
use dusa_collection_utils::functions::{create_hash, truncate};
use dusa_collection_utils::log;
use dusa_collection_utils::log::{set_log_level, LogLevel};
use dusa_collection_utils::rwarc::LockWithTimeout;
use dusa_collection_utils::stringy::Stringy;
use lettre::address::AddressError;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use tokio::net::TcpListener;
mod config;

use std::error::Error;
use std::time::Duration;
use std::{
    io,
    time::Instant,
};

#[derive(Debug, Clone)]
struct TimedEmail {
    email: Email,
    received_at: Instant,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ErrorEmail {
    hash: String,
    subject: Option<String>, // let stream = TcpStream::connect("127.0.0.1:1827").map_err(|e| ErrorArrayItem::from(e))?;
    occoured_at: Instant,
}

#[allow(dead_code)]
fn send_email(config: AppConfig, subject: String, body: String) -> Result<(), ErrorArrayItem> {
    log!(LogLevel::Trace, "Constructing email");
    // Build the email
    let email = Message::builder()
        .to(config.smtp.to.parse().map_err(|e: AddressError| {
            ErrorArrayItem::new(Errors::GeneralError, format!("mailer: {}", e.to_string()))
        })?)
        .from(config.smtp.from.parse().map_err(|e: AddressError| {
            ErrorArrayItem::new(Errors::GeneralError, format!("mailer: {}", e.to_string()))
        })?)
        .subject(subject)
        .body(body)
        .map_err(|e| {
            ErrorArrayItem::new(Errors::GeneralError, format!("mailer: {}", e.to_string()))
        })?;

    // The SMTP credentials
    let creds = Credentials::new(config.smtp.username, config.smtp.password);

    let mailer = SmtpTransport::relay("mail.ramfield.net")
        .map_err(|e| {
            ErrorArrayItem::new(Errors::GeneralError, format!("mailer: {}", e.to_string()))
        })?
        .credentials(creds)
        .build();

    // Send the email
    log!(LogLevel::Trace, "Match statement before sending email");
    let d = match mailer.send(&email) {
        Ok(_) => {
            log!(LogLevel::Info, "Email sent successfully.");
            Ok(())
        }
        Err(e) => {
            log!(LogLevel::Error, "Failed to send email: {}", e);
            Err(ErrorArrayItem::new(
                Errors::GeneralError,
                format!("mailer: {}", e.to_string()),
            ))
        }
    };

    log!(LogLevel::Trace, "Email processed returning");
    d
}

async fn process_emails(
    config: AppConfig,
    emails: LockWithTimeout<Vec<TimedEmail>>,
    errors: LockWithTimeout<Vec<ErrorEmail>>,
    loop_interval: Duration,
    rate_limit: usize,
) {
    loop {
        // Lock the errors vector
        log!(LogLevel::Trace, "Locking email_errors");
        let mut email_errors = match errors.try_write().await {
            Ok(vec) => vec,
            Err(_) => {
                log!(
                    LogLevel::Error,
                    "Failed to acquire write lock on the error counter"
                );
                continue;
            }
        };

        // Lock the emails vector
        log!(LogLevel::Trace, "Locking email_array");
        let mut email_vec = match emails.try_write().await {
            Ok(vec) => vec,
            Err(_) => {
                log!(
                    LogLevel::Error,
                    "Failed to acquire write lock on emails vector"
                );
                email_errors.push(ErrorEmail {
                    hash: truncate(&create_hash("Failed to lock email array".to_owned()), 10)
                        .to_owned(),
                    subject: None,
                    occoured_at: Instant::now(),
                });
                continue;
            }
        };

        log!(LogLevel::Trace, "Starting timeout processing");
        let current_time = Instant::now();
        let mut i = 0;
        let mut iteration_count = 0;
        log!(LogLevel::Trace, "Cloning config for timeout calcs");
        let config_clone = config.clone();
        log!(LogLevel::Trace, "Cloned config: {}", config_clone);

        while i < email_vec.len() && iteration_count < rate_limit {
            if current_time.duration_since(email_vec[i].received_at) > Duration::from_secs(300) {
                log!(
                    LogLevel::Info,
                    "Expired email discarding: {:?}",
                    email_vec[i]
                );
                email_vec.remove(i);
            } else {
                match send_email(
                    config_clone.clone(),
                    email_vec[i].email.subject.to_string(),
                    email_vec[i].email.body.to_string(),
                ) {
                    Ok(_) => {
                        log!(
                            LogLevel::Info,
                            "Sending Email: {} of {}",
                            iteration_count + 1,
                            rate_limit
                        );
                        email_vec.remove(i);
                    }
                    Err(e) => {
                        log!(
                            LogLevel::Error,
                            "An error occurred while sending email: {}",
                            e
                        );
                        email_errors.push(ErrorEmail {
                            hash: truncate(&create_hash(e.to_string()), 10).to_owned(),
                            subject: Some(e.to_string()),
                            occoured_at: Instant::now(),
                        });
                        i += 1;
                    }
                }
            }
            iteration_count += 1;
        }

        if email_errors.is_empty() {
            log!(LogLevel::Info, "No errors reported");
        } else {
            log!(LogLevel::Warn, "Current errors: {}", email_errors.len());
        }

        drop(email_errors);
        drop(email_vec);
        log!(LogLevel::Trace, "Resting");
        // Sleep for the specified interval
        tokio::time::sleep(loop_interval).await;
    }
}


async fn start_server(
    host: &str,
    port: u16,
    emails: LockWithTimeout<Vec<TimedEmail>>,
) -> io::Result<()> {
    let listener = TcpListener::bind(format!("{}:{}", host, port)).await?;
    log!(LogLevel::Info, "Server listening on {}:{}", host, port);

    // let emails_clone = emails.clone();
    loop {
        let emails_clone = emails.clone();
        match listener.accept().await {
            Ok((mut stream, addr)) => {
                log!(LogLevel::Info, "Accepted connection from {}", addr);
                tokio::spawn(async move {
                    match receive_message_tcp::<Stringy>(&mut stream).await {
                        Ok(message) => {
                            let email = Email::from_json(&message.payload)
                                .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

                            log!(LogLevel::Info, "You've got mail: {}", email);

                            // Add email to the vector with current timestamp
                            let timed_email = TimedEmail {
                                email: email.clone(),
                                received_at: Instant::now(),
                            };

                            emails_clone
                                .try_write_with_timeout(Some(Duration::from_secs(3)))
                                .await
                                .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?
                                .push(timed_email);

                            drop(emails_clone);

                            return Ok(())

                        }
                        Err(err) => return Err(err),
                    }
                })
            }
            Err(err) => return Err(err),
        };
    }
}

fn load_app_config() -> Result<AppConfig, Box<dyn Error>> {
    let settings = Config::builder()
        .add_source(File::with_name("Config"))
        .build()?;

    settings.try_deserialize().map_err(|e| e.into())
}

#[tokio::main]
async fn main() {
    // Load the application configuration
    let app_config: AppConfig = match load_app_config() {
        Ok(config) => config,
        Err(e) => {
            log!(LogLevel::Error, "Failed to load configuration: {}", e);
            return;
        }
    };

    let default_config = match artisan_middleware::config::AppConfig::new() {
        Ok(mut data_loaded) => {
            data_loaded.git = None;
            data_loaded.database = None;
            data_loaded.app_name = Stringy::from_string(env!("CARGO_PKG_NAME").to_string());
            data_loaded.version = env!("CARGO_PKG_VERSION").to_string();
            data_loaded
        }
        Err(e) => {
            log!(LogLevel::Error, "Error loading config: {}", e);
            return;
        }
    };

    // Set the log level dynamically based on the configuration or default
    set_log_level(default_config.log_level);
    log!(
        LogLevel::Info,
        "Server starting with log level: {:?}",
        default_config.log_level
    );

    if default_config.debug_mode {
        log!(LogLevel::Info, "{default_config}");
        log!(LogLevel::Info, "{app_config}");
    };

    // Set up loop interval and rate limit from configuration
    let loop_interval_seconds = app_config.app.loop_interval_seconds;
    let rate_limit = app_config.app.rate_limit;

    log!(
        LogLevel::Info,
        "Loop interval set to: {} seconds",
        loop_interval_seconds
    );
    log!(LogLevel::Info, "Rate limit set to: {}", rate_limit);

    // Vector to store emails
    let emails: LockWithTimeout<Vec<TimedEmail>> = LockWithTimeout::new(Vec::new());
    let errors: LockWithTimeout<Vec<ErrorEmail>> = LockWithTimeout::new(Vec::new());

    // Start the email processing loop in a separate thread
    let emails_clone: LockWithTimeout<Vec<TimedEmail>> = emails.clone();
    let errors_clone: LockWithTimeout<Vec<ErrorEmail>> = errors.clone();
    let loop_interval = Duration::from_secs(loop_interval_seconds);
    let app_config_clone: AppConfig = app_config.clone();
    tokio::spawn(async move {
        process_emails(
            app_config_clone,
            emails_clone,
            errors_clone,
            loop_interval,
            rate_limit,
        )
        .await;
    });
    // Start the server
    if let Err(err) = start_server("0.0.0.0", 1827, emails).await {
        log!(LogLevel::Error, "Error running server: {}", err);
    }
}
