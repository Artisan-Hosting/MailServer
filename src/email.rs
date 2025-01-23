
use dusa_collection_utils::{errors::{ErrorArrayItem, Errors}, log::LogLevel, log};
use lettre::{address::AddressError, transport::smtp::authentication::Credentials, Message, SmtpTransport, Transport};

use crate::config::AppConfig;

pub fn send_email(config: &AppConfig, subject: String, body: String) -> Result<(), ErrorArrayItem> {
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
    let creds = Credentials::new(config.smtp.username.to_owned(), config.smtp.password.to_owned());

    let mailer = SmtpTransport::relay("mail.ramfield.net")
        .map_err(|e| {
            ErrorArrayItem::new(Errors::GeneralError, format!("mailer: {}", e.to_string()))
        })?
        .credentials(creds)
        .build();

    // Send the email
    log!(LogLevel::Trace, "Match statement before sending email");
    match mailer.send(&email) {
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
    }
}