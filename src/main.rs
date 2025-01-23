use ::config::{Config, File};
use artisan_middleware::common::{update_state, wind_down_state};
use artisan_middleware::notifications::Email;
use artisan_middleware::state_persistence::{AppState, StatePersistence};
use artisan_middleware::timestamp::current_timestamp;
use artisan_middleware::version::{aml_version, str_to_version};
use config::AppConfig;
use dusa_collection_utils::errors::{ErrorArrayItem, UnifiedResult};
use dusa_collection_utils::functions::{create_hash, truncate};
use dusa_collection_utils::log;
use dusa_collection_utils::log::{set_log_level, LogLevel};
use dusa_collection_utils::rwarc::LockWithTimeout;
use dusa_collection_utils::stringy::Stringy;
use dusa_collection_utils::types::PathType;
use dusa_collection_utils::version::{SoftwareVersion, Version, VersionCode};
use email::send_email;
use signals::{reload_monitor, shutdown_monitor};
use simple_comms::network::send_receive::send_empty_ok;
use simple_comms::protocol::flags::Flags;
use simple_comms::protocol::header::{ProtocolHeader, EOL};
use simple_comms::protocol::io_helpers::read_until;
use simple_comms::protocol::message::ProtocolMessage;
use simple_comms::protocol::proto::Proto;
use simple_comms::protocol::status::ProtocolStatus;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, RwLockWriteGuard};
use tokio::time::sleep;
mod config;
mod email;
mod signals;
use core::panic;
use std::error::Error;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone)]
struct TimedEmail {
    email: Email,
    received_at: Instant,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ErrorEmail {
    hash: Stringy,
    subject: Option<String>, // let stream = TcpStream::connect("127.0.0.1:1827").map_err(|e| ErrorArrayItem::from(e))?;
    occoured_at: Instant,
}

const PORT: u16 = 1827;
const HOST: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

#[tokio::main]
async fn main() {
    // Load the application configurations
    let app_config: AppConfig = match load_app_config() {
        Ok(config) => config,
        Err(e) => {
            log!(LogLevel::Error, "Failed to load configuration: {}", e);
            std::process::exit(0)
        }
    };

    let default_config = match artisan_middleware::config::AppConfig::new() {
        Ok(mut data_loaded) => {
            data_loaded.git = None;
            data_loaded.database = None;
            data_loaded.app_name = Stringy::from(env!("CARGO_PKG_NAME").to_string());

            let raw_version: SoftwareVersion = {
                // defining the version
                let library_version: Version = aml_version();
                let software_version: Version =
                    str_to_version(env!("CARGO_PKG_VERSION"), Some(VersionCode::Production));

                SoftwareVersion {
                    application: software_version,
                    library: library_version,
                }
            };

            data_loaded.version =
                serde_json::to_string(&raw_version).unwrap_or(SoftwareVersion::dummy().to_string());

            data_loaded
        }
        Err(e) => {
            log!(LogLevel::Error, "Error loading config: {}", e);
            // return;
            panic!()
        }
    };

    //  Initialize app state
    let state_path: PathType = StatePersistence::get_state_path(&default_config);
    let mut state = match StatePersistence::load_state(&state_path).await {
        Ok(mut loaded_data) => {
            log!(LogLevel::Info, "Loaded previous state data");
            log!(LogLevel::Trace, "Previous state data: {:#?}", loaded_data);
            loaded_data.is_active = false;
            loaded_data.data = String::from("Initializing");
            loaded_data.version = {
                let library: Version = aml_version();
                let application = Version::new(env!("CARGO_PKG_VERSION"), VersionCode::Production);

                SoftwareVersion {
                    application,
                    library,
                }
            };
            loaded_data.config.debug_mode = default_config.debug_mode;
            loaded_data.last_updated = current_timestamp();
            loaded_data.config.log_level = default_config.log_level;
            set_log_level(loaded_data.config.log_level);
            loaded_data.error_log.clear();
            update_state(&mut loaded_data, &state_path, None).await;
            loaded_data
        }
        Err(e) => {
            log!(LogLevel::Warn, "No previous state loaded, creating new one");
            log!(LogLevel::Debug, "Error loading previous state: {}", e);
            let mut state = AppState {
                name: env!("CARGO_PKG_NAME").to_owned(),
                version: {
                    let library: Version = aml_version();
                    let application =
                        Version::new(env!("CARGO_PKG_VERSION"), VersionCode::Production);

                    SoftwareVersion {
                        application,
                        library,
                    }
                },
                data: String::new(),
                last_updated: current_timestamp(),
                event_counter: 0,
                is_active: false,
                error_log: vec![],
                config: default_config.clone(),
                system_application: true,
            };
            state.is_active = false;
            state.data = String::from("Initializing");
            state.config.debug_mode = true;
            state.last_updated = current_timestamp();
            state.config.log_level = default_config.log_level;
            set_log_level(LogLevel::Trace);
            state.error_log.clear();
            update_state(&mut state, &state_path, None).await;
            state
        }
    };

    set_log_level(LogLevel::Trace);

    // Listening for the signals
    let reload_flag = Arc::new(Notify::new());
    let shutdown_flag = Arc::new(Notify::new());
    let execution: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));

    // Spawn separate tasks that might signal to the main loop
    let reload_flag_clone = reload_flag.clone();
    reload_monitor(reload_flag_clone);

    let shutdown_flag_clone = shutdown_flag.clone();
    shutdown_monitor(shutdown_flag_clone);

    // Arrays to store email data and errors
    let emails: LockWithTimeout<Vec<TimedEmail>> = LockWithTimeout::new(Vec::new());
    let errors: LockWithTimeout<Vec<ErrorEmail>> = LockWithTimeout::new(Vec::new());

    // Defining the listeners
    let tcp_listener: TcpListener = UnifiedResult::new(
        TcpListener::bind(format!("{}:{}", HOST, PORT))
            .await
            .map_err(|err| ErrorArrayItem::from(err)),
    )
    .unwrap();

    loop {
        tokio::select! {
            Ok(mut conn) = tcp_listener.accept() => {

                let mut response: ProtocolMessage<()> =
                UnifiedResult::new(ProtocolMessage::new(Flags::NONE, ()).map_err(ErrorArrayItem::from))
                .unwrap();

                if execution.load(Ordering::Relaxed) {
                      // ? To allow for response sending based on messages getting all the way into the locked array we're implementing the receiver logic here
                        // Read until EOL to get the entire message
                        let mut buffer: Vec<u8> = UnifiedResult::new(
                            read_until(&mut conn.0, EOL.to_vec())
                                .await
                                .map_err(|err| ErrorArrayItem::from(err)),
                        )
                        .unwrap();

                        // Truncate the EOL from the buffer
                        if let Some(pos) = buffer
                            .windows(EOL.len())
                            .rposition(|window| window == EOL.to_vec())
                        {
                            buffer.truncate(pos);
                        }

                        match ProtocolMessage::<Stringy>::from_bytes(&buffer).await {
                            Ok(message) => {
                                log!(LogLevel::Debug, "Message recieved: {:#?}", message);
                                let header: ProtocolHeader = message.header;

                                if header.flags != Flags::OPTIMIZED.bits() {
                                    // Preparing a response requesting a resend with a upgrade

                                    response.header.status = ProtocolStatus::SIDEGRADE.bits();
                                    response.header.reserved = Flags::OPTIMIZED.bits();
                                    log!(LogLevel::Error, "Recieved message in a illegal format asking them to try again");
                                    log!(
                                        LogLevel::Debug,
                                        "Sent the following header to sender: {}",
                                        response.header
                                    );

                                    let response_bytes: Vec<u8> = UnifiedResult::new(
                                        response.to_bytes().await.map_err(ErrorArrayItem::from),
                                    )
                                    .unwrap();

                                    let _ = conn.0.write_all(&response_bytes).await;
                                    let _ = conn.0.flush().await;
                                    state.event_counter += 1;
                                    update_state(&mut state, &state_path, None).await;

                                    // log!(Log)
                                    continue;
                                }

                                // ! Now were processing the email data
                                let payload: Stringy = message.payload;

                                let email: Email = match Email::from_json(&payload) {
                                    Ok(email) => email,
                                    Err(err) => {
                                        log!(
                                            LogLevel::Error,
                                            "Error while deserializing email: {}",
                                            err
                                        );

                                        send_err_tcp(&mut conn.0).await;
                                        state.event_counter += 1;
                                        update_state(&mut state, &state_path, None).await;
                                        continue;
                                    }
                                };

                                // preping email for queue
                                let email_tagged = TimedEmail {
                                    email,
                                    received_at: Instant::now(),
                                };

                                let email_array_results: UnifiedResult<
                                    RwLockWriteGuard<'_, Vec<TimedEmail>>,
                                > = UnifiedResult::new(
                                    emails.try_write_with_timeout(None).await,
                                );

                                if email_array_results.is_err() {
                                    send_err_tcp(&mut conn.0).await;
                                    // continue;
                                    panic!()
                                }

                                let mut email_array: RwLockWriteGuard<'_, Vec<TimedEmail>> =
                                    email_array_results.unwrap();

                                {
                                    email_array.push(email_tagged);
                                    drop(email_array);
                                }

                                let _ = send_empty_ok::<TcpStream>(&mut conn.0, Proto::TCP).await.unwrap();

                                state.event_counter += 1;
                                update_state(&mut state, &state_path, None).await;
                            }
                            Err(error) => {
                                response.header.status = ProtocolStatus::ERROR.bits();
                                let response_bytes: Vec<u8> = UnifiedResult::new(
                                    response.to_bytes().await.map_err(ErrorArrayItem::from),
                                )
                                .unwrap();

                                let _ = conn.0.write_all(&response_bytes).await;
                                let _ = conn.0.flush().await;

                                state.event_counter += 1;
                                update_state(&mut state, &state_path, None).await;

                                log!(LogLevel::Error, "Error reading message: {}", error);
                                continue;
                            }
                        }

                }


            },
            _ = reload_flag.notified() => {
                execution.store(false, Ordering::Relaxed);
                // sleep to ensure the other threads paused execution
                sleep(Duration::from_secs(2)).await;

                // if a reload is called, we'll clear the message queue and reload the config data
                update_state(&mut state, &state_path, None).await;

                let mut email_array =
                    UnifiedResult::new(emails.try_write_with_timeout(None).await)
                        .unwrap();

                email_array.clear();
                drop(email_array);

                // Load the application configuration
                let default_config = match artisan_middleware::config::AppConfig::new() {
                    Ok(mut data_loaded) => {
                        data_loaded.git = None;
                        data_loaded.database = None;
                        data_loaded.app_name =
                            Stringy::from(env!("CARGO_PKG_NAME").to_string());
                        // data_loaded.version = env!("CARGO_PKG_VERSION").to_string();
                        data_loaded
                    }
                    Err(e) => {
                        log!(LogLevel::Error, "Error loading config: {}", e);
                        // return;
                        panic!()
                    }
                };

                // Initialize app state
                let mut state = match StatePersistence::load_state(&state_path).await {
                    Ok(mut loaded_data) => {
                        log!(LogLevel::Info, "Loaded previous state data");
                        log!(LogLevel::Trace, "Previous state data: {:#?}", loaded_data);
                        loaded_data.is_active = false;
                        loaded_data.data = String::from("Initializing");
                        loaded_data.config.debug_mode = default_config.debug_mode;
                        loaded_data.last_updated = current_timestamp();
                        loaded_data.config.log_level = default_config.log_level;
                        set_log_level(loaded_data.config.log_level);
                        loaded_data.error_log.clear();
                        loaded_data
                    }
                    Err(e) => {
                        log!(LogLevel::Warn, "No previous state loaded, creating new one");
                        log!(LogLevel::Debug, "Error loading previous state: {}", e);
                        let mut state = AppState {
                            name: env!("CARGO_PKG_NAME").to_owned(),
                            version: {
                                let library: Version = aml_version();
                                let application = Version::new(env!("CARGO_PKG_VERSION"), VersionCode::Production);

                                SoftwareVersion{ application, library }
                            },
                            data: String::new(),
                            last_updated: current_timestamp(),
                            event_counter: 0,
                            is_active: false,
                            error_log: vec![],
                            config: default_config.clone(),
                            system_application: true
                        };
                        state.is_active = false;
                        state.data = String::from("Initializing");
                        state.config.debug_mode = true;
                        state.last_updated = current_timestamp();
                        state.config.log_level = default_config.log_level;
                        set_log_level(LogLevel::Trace);
                        state.error_log.clear();

                        state
                    }
                };

                update_state(&mut state, &state_path, None).await;

                execution.store(true, Ordering::Relaxed);
            },
            _ = shutdown_flag.notified() => {
                execution.store(false, Ordering::Relaxed);
                // sleep to ensure the other threads paused execution
                sleep(Duration::from_secs(2)).await;
                wind_down_state(&mut state, &state_path).await;
                std::process::exit(0);

            },
            _ = sleep(Duration::from_secs(app_config.app.loop_interval_seconds)) => {
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
                            hash: truncate(&*create_hash("Failed to lock email array".to_owned()), 10),
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

                while i < email_vec.len() && iteration_count < app_config.app.rate_limit {
                    if current_time.duration_since(email_vec[i].received_at) > Duration::from_secs(300) {
                        log!(
                            LogLevel::Info,
                            "Expired email discarding: {:?}",
                            email_vec[i]
                        );
                        email_vec.remove(i);
                    } else {
                        match send_email(
                            &app_config,
                            email_vec[i].email.subject.to_string(),
                            email_vec[i].email.body.to_string(),
                        ) {
                            Ok(_) => {
                                log!(
                                    LogLevel::Info,
                                    "Sending Email: {} of {}",
                                    iteration_count + 1,
                                    app_config.app.rate_limit
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
                                    hash: truncate(&*create_hash(e.to_string()), 10).to_owned(),
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
                    log!(LogLevel::Debug, "No errors reported");
                } else {
                    log!(LogLevel::Warn, "Current errors: {}", email_errors.len());
                }

                drop(email_errors);
                drop(email_vec);
                log!(LogLevel::Trace, "Resting");
            },
        }
    }

    // Sending error over tcp
    async fn send_err_tcp(conn: &mut TcpStream) {
        let mut response: ProtocolMessage<()> =
            UnifiedResult::new(ProtocolMessage::new(Flags::NONE, ()).map_err(ErrorArrayItem::from))
                .unwrap();

        response.header.status = ProtocolStatus::ERROR.bits();

        let response_bytes: Vec<u8> =
            UnifiedResult::new(response.to_bytes().await.map_err(ErrorArrayItem::from)).unwrap();

        let _ = conn.write_all(&response_bytes).await;
        let _ = conn.flush().await;
        // return;
        panic!();
    }
}

fn load_app_config() -> Result<AppConfig, Box<dyn Error>> {
    let settings = Config::builder()
        .add_source(File::with_name("Config"))
        .build()?;

    settings.try_deserialize().map_err(|e| e.into())
}
