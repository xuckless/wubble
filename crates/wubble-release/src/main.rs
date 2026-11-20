//! `wubble-release`: the update key, signatures and the manifest (docs/release.md).
//!
//!     wubble-release keygen --out DIR [--password-env VAR]
//!     wubble-release sign --key-env VAR [--password-env VAR] FILE...
//!     wubble-release make-manifest --version V --base-url URL [--notes URL] --out FILE DIR
//!
//! Secrets come only from the environment variables named, never from the command line.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use wubble_release::{Key, Version, keygen, make_manifest};

#[derive(Parser)]
#[command(name = "wubble-release", about = "wubble's release tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make the update key pair: DIR/wubble-update.key (secret) and DIR/wubble-update.pub.
    Keygen {
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Encrypt the secret key with the password in this environment variable.
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
    },
    /// Write FILE.minisig for each FILE.
    Sign {
        /// The environment variable holding the secret key file's text.
        #[arg(long, value_name = "VAR")]
        key_env: String,
        /// The environment variable holding its password, if it is encrypted (unset or empty:
        /// not encrypted).
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Write the update manifest from DIR's wubble-V-<platform>.tar.xz and their .minisig.
    MakeManifest {
        #[arg(long)]
        version: Version,
        /// Where the archives are downloaded from (the release's download URL).
        #[arg(long, value_name = "URL")]
        base_url: String,
        /// The release page (default: derived from the base URL).
        #[arg(long, value_name = "URL")]
        notes: Option<String>,
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        dir: PathBuf,
    },
}

fn env_secret(var: &str) -> Result<String, String> {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(format!("the environment variable {var} is empty or unset")),
    }
}

fn password(var: Option<&str>) -> Option<String> {
    var.and_then(|v| std::env::var(v).ok())
        .filter(|p| !p.is_empty())
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Keygen { out, password_env } => {
            let pw = match &password_env {
                Some(var) => Some(env_secret(var)?),
                None => None,
            };
            let key = keygen(pw)?;
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let secret = out.join("wubble-update.key");
            if secret.exists() {
                return Err(format!("{} exists; not overwritten", secret.display()));
            }
            write_private(&secret, &key.secret)?;
            std::fs::write(out.join("wubble-update.pub"), &key.public)
                .map_err(|e| e.to_string())?;
            println!("wrote {} and wubble-update.pub", secret.display());
            println!("public key (for update::key::PUBLIC_KEY and install.sh):");
            println!("{}", key.base64);
        }
        Command::Sign {
            key_env,
            password_env,
            files,
        } => {
            let key = Key::from_secret(&env_secret(&key_env)?, password(password_env.as_deref()))?;
            for file in files {
                let name = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| format!("{}: no file name", file.display()))?
                    .to_owned();
                let data = std::fs::File::open(&file).map_err(|e| format!("{name}: {e}"))?;
                let sig = key.sign(std::io::BufReader::new(data), &name)?;
                let out = file.with_file_name(format!("{name}.minisig"));
                std::fs::write(&out, sig).map_err(|e| format!("{}: {e}", out.display()))?;
                println!("signed {name}");
            }
        }
        Command::MakeManifest {
            version,
            base_url,
            notes,
            out,
            dir,
        } => {
            let notes = notes.unwrap_or_else(|| base_url.replacen("/download/", "/tag/", 1));
            let date = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let manifest = make_manifest(&version, &base_url, &notes, &dir, &date)?;
            std::fs::write(&out, manifest.to_json())
                .map_err(|e| format!("{}: {e}", out.display()))?;
            let platforms: Vec<&str> = manifest.platforms.keys().map(String::as_str).collect();
            println!(
                "wrote {} for {version}: {}",
                out.display(),
                platforms.join(", ")
            );
        }
    }
    Ok(())
}

/// The secret key, readable by its owner only.
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wubble-release: {e}");
            ExitCode::FAILURE
        }
    }
}
