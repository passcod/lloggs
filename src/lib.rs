//! Logging configuration for [clap] applications.
//!
//! This library provides a common set of flags for controlling logging in a CLI application, and
//! a default implementation for configuring logging based on those flags using non-blocking
//! [tracing-subscriber](https://docs.rs/tracing-subscriber) when the `tracing` feature is enabled
//! (which is the default).
//!
//! It also supports configuring logging before parsing arguments, to allow logging to be set up
//! using environment variables such as `RUST_LOG` or `DEBUG_INVOCATION`, respects the `NO_COLOR`
//! (<https://no-color.org>) and `CLICOLOR_FORCE` (<https://bixense.com/clicolors/>) environment
//! variables, and adjusts defaults when it detects systemd.
//!
//! # Example
//!
//! ```no_run
//! use lloggs::{LoggingArgs, PreArgs};
//! use clap::Parser;
//!
//! #[derive(Debug, Parser)]
//! struct Args {
//!     #[command(flatten)]
//!     logging: LoggingArgs,
//!
//!     // Your other arguments here
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     let mut _guard = PreArgs::parse().setup()?;
//!     let args = Args::parse();
//!     if _guard.is_none() {
//!         _guard = Some(args.logging.setup(|v| match v {
//!             0 => "info",
//!             1 => "debug",
//!             _ => "trace",
//!         })?);
//!     }
//!
//!     // Your application logic here
//!
//!     Ok(())
//! }
//! ```
//!
//! # Features
//!
//! ## `tracing` (default)
//!
//! Include functionality to instantiate a [`tracing_subscriber`] based on the logging options.
//!
//! ## `miette-7`
//!
//! Return [`miette::Report`]s instead of Boxed errors.

use std::{env::var, ffi::OsStr, io::IsTerminal, path::PathBuf};

use clap::{ArgAction, Parser, ValueEnum, ValueHint};

#[cfg(feature = "tracing")]
pub use tracing_appender::non_blocking::WorkerGuard;

#[cfg(feature = "miette-7")]
type Error = miette::Report;
#[cfg(not(feature = "miette-7"))]
type Error = Box<dyn std::error::Error + Sync + Send>;

type Result<T> = std::result::Result<T, Error>;

fn string_err<T>(err: &str) -> Result<T> {
	#[cfg(feature = "miette-7")]
	{
		Err(miette::miette!("{err}"))
	}

	#[cfg(not(feature = "miette-7"))]
	{
		Err(err.into())
	}
}

fn tracing_err(err: Box<dyn std::error::Error + Send + Sync>) -> Error {
	#[cfg(feature = "miette-7")]
	{
		miette::Report::from_err(Box::leak(err) as &_)
	}

	#[cfg(not(feature = "miette-7"))]
	{
		err
	}
}

/// Clap flags that control logging.
///
/// This struct implements clap's [`Parser`] trait, so it can be injected into any
/// clap command derive.
///
/// The field documentation is used as the help message for each flag; this doc-comment is
/// ignored by clap as it's imported via `#[command(flatten)]`.
///
/// # Example
///
/// ```rust
/// use lloggs::LoggingArgs;
/// use clap::Parser;
///
/// #[derive(Debug, Parser)]
/// struct Args {
///     #[command(flatten)]
///     logging: LoggingArgs,
///
///     // Your other arguments here
/// }
/// ```
///
/// This will add the following flags to your command:
///
/// ```plain
/// --color <MODE>       When to use terminal colours [default: auto]
/// -v, --verbose...     Set diagnostic log level
/// --log-file [<PATH>]  Write diagnostic logs to a file
/// --log-timeless       Omit timestamps in logs
/// ```
///
/// You should then use [`LoggingArgs::setup()`] to configure logging.
#[derive(Debug, Clone, Parser)]
pub struct LoggingArgs {
	/// When to use terminal colours.
	///
	/// You can also set the `NO_COLOR` environment variable to disable colours, or the
	/// `CLICOLOR_FORCE` environment variable to force colours. Defaults to `auto`, which
	/// checks whether the output is a terminal to decide.
	#[arg(long, default_value = "auto", value_name = "MODE", alias = "colour")]
	pub color: ColourMode,

	/// Set diagnostic log level.
	///
	/// This enables diagnostic logging, which is useful for investigating bugs. Use multiple
	/// times to increase verbosity.
	///
	/// You may want to use with `--log-file` to avoid polluting your terminal.
	#[arg(
		long,
		short,
		action = ArgAction::Count,
		num_args = 0,
		default_value = "0",
	)]
	pub verbose: u8,

	/// Write diagnostic logs to a file.
	///
	/// This writes diagnostic logs to a file, instead of the terminal, in JSON format.
	///
	/// If the path provided is a directory, a file will be created in that directory. The file name
	/// will be the current date and time, in the format `programname.YYYY-MM-DDTHH-MM-SSZ.log`.
	#[arg(
		long,
		num_args = 0..=1,
		default_missing_value = ".",
		value_hint = ValueHint::AnyPath,
		value_name = "PATH",
	)]
	pub log_file: Option<PathBuf>,

	/// Omit timestamps in logs.
	///
	/// This can be useful when running under service managers that capture logs, to avoid having
	/// two timestamps. When run under systemd, this is automatically enabled.
	///
	/// This option is ignored if the log file is set, or when using `RUST_LOG` or equivalent
	/// (as logging is initialized before arguments are parsed in that case); you may want to use
	/// `LOG_TIMELESS` instead in the latter case.
	#[arg(long)]
	pub log_timeless: bool,
}

impl LoggingArgs {
	/// Configure logging according to arguments.
	///
	/// This uses a non-blocking [tracing-subscriber](tracing_subscriber) logger. It returns a guard
	/// that must be kept alive for the duration of the program, to ensure that logs are output.
	///
	/// This MUST NOT be called if logging has been previously configured, such as when
	/// [`PreArgs::setup()`] returned `Some`.
	///
	/// # Level mapping
	///
	/// The `level_map` function is called with the verbosity level, and should return a logline string
	/// that [tracing-subscriber][1] can interpret as a `RUST_LOG` filter. For example:
	///
	/// [1]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html
	///
	/// ```ignore
	/// |v| match v {
	///     0 => "info",
	///     1 => "info,yourprog=debug",
	///     2 => "debug",
	///     3 => "debug,yourprog=trace",
	///     _ => "trace",
	/// }
	/// ```
	///
	/// # Panics
	///
	/// Panics in debug mode if colours are enabled or automatic, but the `ansi` feature is not
	/// enabled on the `tracing-subscriber` dependency.
	///
	/// Panics if logging cannot be initialised.
	#[cfg(feature = "tracing")]
	pub fn setup(&self, level_map: impl FnOnce(u8) -> &'static str) -> Result<WorkerGuard> {
		use std::{env::current_exe, fs::metadata, io::stderr};
		use time::{macros::format_description, OffsetDateTime};
		use tracing_appender::{non_blocking, rolling};

		let (log_writer, guard) = if let Some(file) = &self.log_file {
			let is_dir = metadata(file).is_ok_and(|info| info.is_dir());
			let (dir, filename) = if is_dir {
				let progname = current_exe()
					.ok()
					.and_then(|path| {
						path.file_stem()
							.map(|stem| stem.to_string_lossy().to_string())
					})
					.unwrap_or(env!("CARGO_PKG_NAME").into());

				let time = OffsetDateTime::now_utc()
					.format(format_description!(
						"[year]-[month]-[day]T[hour]-[minute]-[second]Z"
					))
					.unwrap_or("debug".into());
				(
					file.to_owned(),
					PathBuf::from(format!("{progname}.{time}.log",)),
				)
			} else if let (Some(parent), Some(file_name)) = (file.parent(), file.file_name()) {
				(parent.into(), PathBuf::from(file_name))
			} else {
				return string_err("Failed to determine log file name");
			};

			non_blocking(rolling::never(dir, filename))
		} else {
			non_blocking(stderr())
		};

		let color = ColourMode::from_env().or_if_auto(self.color);

		let timeless = is_systemd() || self.log_timeless;

		let mut builder = tracing_subscriber::fmt()
			.with_env_filter(level_map(self.verbose))
			.with_ansi(color.enabled())
			.with_writer(log_writer);

		if self.verbose > 0 {
			use tracing_subscriber::fmt::format::FmtSpan;
			builder = builder.with_span_events(FmtSpan::NEW | FmtSpan::CLOSE);
		}

		if self.log_file.is_some() {
			builder.json().init();
		} else if timeless {
			builder.without_time().init();
		} else {
			builder.init();
		}

		Ok(guard)
	}
}

/// Logging configuration obtained before parsing arguments.
#[derive(Debug, Clone)]
pub struct PreArgs {
	/// A `RUST_LOG` format logging configuration line.
	///
	/// This is typically interpreted by [env\_logger][1] or [tracing-subscriber][2].
	///
	/// No format is enforced by this library, except that the presence of `DEBUG_INVOCATION` is
	/// treated as `RUST_LOG=debug`.
	///
	/// [1]: https://docs.rs/env_logger/latest/env_logger/#enabling-logging
	/// [2]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html
	pub logline: Option<String>,

	/// Whether to include timestamps in logs.
	///
	/// This is set to `true` if any of the following environment variables are present:
	/// - `JOURNAL_STREAM` (indicating systemd) if stderr isn't a terminal
	/// - `DEBUG_INVOCATION` (indicating systemd in debug mode)
	/// - `LOG_TIMELESS` (custom)
	pub timeless: bool,

	/// Whether to colourise terminal output.
	///
	/// This is set to `None` if the `NO_COLOR` environment variable is set or to `Always` if the
	/// `CLICOLOR_FORCE` variable is set.
	pub color: ColourMode,
}

impl PreArgs {
	/// Obtain logging options before parsing arguments, from the given environment variable.
	///
	/// This should be called before parsing arguments, to optionally obtain logging configuration
	/// before parsing arguments. This is useful for setting up logging early, so that it can be
	/// used to log errors during argument parsing and interpretation.
	///
	/// To configure logging, call [`setup()`][PreArgs::setup()] on the returned value if using the
	/// default setup, or interpret the fields manually if you need to do something more custom.
	///
	/// The environment variable from the name given is read (hence called the "logline"), it must be
	/// in `RUST_LOG` format. If your application is able to launch or supervise sub-programs which
	/// may themselves interpret the `RUST_LOG` variable, it's preferable to use a different name that
	/// matches your application here instead, so that logging can be targeted more precisely.
	///
	/// The `DEBUG_INVOCATION` environment variable [may be set][1] by systemd [since v257][2]; if
	/// it is present, this is equivalent to setting the logline to `debug`. If the logline variable
	/// is present too, `DEBUG_INVOCATION` is ignored.
	///
	/// [1]: https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#RestartMode=
	/// [2]: https://mastodon.social/@pid_eins/113548780685011324
	pub fn parse_with_env(var_name: impl AsRef<OsStr>) -> Self {
		let logline = var(var_name).ok().or_else(|| {
			if var("DEBUG_INVOCATION").is_ok() {
				Some("debug".into())
			} else {
				None
			}
		});

		let timeless = is_systemd() || var("LOG_TIMELESS").is_ok();

		let color = ColourMode::from_env();

		Self {
			logline,
			timeless,
			color,
		}
	}

	/// Obtain logging options before parsing arguments.
	///
	/// This should be called before parsing arguments, to optionally obtain logging configuration
	/// before parsing arguments. This is useful for setting up logging early, so that it can be
	/// used to log errors during argument parsing and interpretation.
	///
	/// To configure logging, call [`setup()`][PreArgs::setup()] on the returned value if using the
	/// default setup, or interpret the fields manually if you need to do something more custom.
	///
	/// The `DEBUG_INVOCATION` environment variable [may be set][1] by systemd [since v257][2]; if it
	/// is present, this is equivalent to setting `RUST_LOG=debug`. If `RUST_LOG` is set, it takes
	/// precedence.
	///
	/// [1]: https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#RestartMode=
	/// [2]: https://mastodon.social/@pid_eins/113548780685011324
	pub fn parse() -> Self {
		Self::parse_with_env("RUST_LOG")
	}

	/// Configure logging if the logline is set.
	///
	/// This uses a non-blocking [tracing-subscriber](tracing_subscriber) logger. It returns a guard
	/// that must be kept alive for the duration of the program, to ensure that logs are output.
	///
	/// If `logline` is `None`, this does nothing and returns `Ok(None)`. See `parse_with_env` for
	/// more details.
	///
	/// Panics in debug mode if colours are enabled or automatic, but the `ansi` feature is not
	/// enabled on the `tracing-subscriber` dependency.
	#[cfg(feature = "tracing")]
	pub fn setup(&self) -> Result<Option<WorkerGuard>> {
		use std::io::stderr;
		use tracing_appender::non_blocking;
		use tracing_subscriber::EnvFilter;

		let Some(logline) = self.logline.as_ref() else {
			return Ok(None);
		};

		let (writer, guard) = non_blocking(stderr());

		let sub = tracing_subscriber::fmt()
			.with_ansi(self.color.enabled())
			.with_env_filter(EnvFilter::new(logline))
			.with_writer(writer);

		if self.timeless {
			sub.without_time()
				.try_init()
				.map(|_| Some(guard))
				.map_err(tracing_err)
		} else {
			sub.try_init().map(|_| Some(guard)).map_err(tracing_err)
		}
	}
}

/// Colour mode for terminal output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ColourMode {
	/// Automatically detect whether to use colours.
	#[default]
	Auto,

	/// Always use colours, even if the terminal does not support them.
	Always,

	/// Never use colours.
	Never,
}

impl ColourMode {
	/// Override only if the current value is auto.
	pub(crate) fn or_if_auto(self, value: Self) -> Self {
		match self {
			Self::Auto => value,
			other => other,
		}
	}

	/// Whether to use colours.
	pub fn enabled(self) -> bool {
		match self {
			Self::Auto => is_terminal(),
			Self::Always => true,
			Self::Never => false,
		}
	}

	/// Override if `NO_COLOR` is set.
	///
	/// Checks if the `NO_COLOR` environment variable is set, and returns `ColourMode::Never` if so.
	///
	/// This is compliant with <https://no-color.org>.
	#[deprecated(since = "1.2.0", note = "use from_env instead")]
	pub fn with_env(self) -> Self {
		if var("NO_COLOR").is_ok() {
			Self::Never
		} else {
			self
		}
	}

	/// Reads the environment and creates a default value.
	///
	/// This complies with <https://bixense.com/clicolors/>.
	/// `CLICOLOR` is ignored because the default is always `Auto`.
	pub fn from_env() -> Self {
		if var("NO_COLOR").is_ok() {
			Self::Never
		} else if var("CLICOLOR_FORCE").is_ok() {
			Self::Always
		} else if enable_ansi_support::enable_ansi_support().is_err() {
			Self::Never
		} else {
			Self::Auto
		}
	}

	/// Override if ANSI cannot be enabled on Windows.
	///
	/// Tries to enable ANSI colour support on Windows, and returns `ColourMode::Never` if it fails.
	///
	/// This is a no-op on non-Windows platforms, or if `ColourMode::Never` is already set.
	#[deprecated(since = "1.2.0", note = "use from_env instead")]
	pub fn with_windows(self) -> Self {
		match self {
			Self::Never => Self::Never,
			mode => {
				if enable_ansi_support::enable_ansi_support().is_err() {
					Self::Never
				} else {
					mode
				}
			}
		}
	}
}

fn is_terminal() -> bool {
	std::io::stderr().is_terminal()
}

fn is_systemd() -> bool {
	(var("JOURNAL_STREAM").is_ok() && !is_terminal()) || var("DEBUG_INVOCATION").is_ok()
}
