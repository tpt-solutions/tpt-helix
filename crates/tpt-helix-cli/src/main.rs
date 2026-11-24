//! `helix` binary: argument parsing and output. Behavior lives in `tpt_helix_cli`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpt_helix_cli::{
    CliError, all_required_ok, build_guest, doctor_checks, migrate_html, migration_report,
    migration_warnings, new_project, parse_grants, render_html, run_module, write_guest_crate,
};

#[derive(Parser)]
#[command(name = "helix", version, about = "TPT Helix developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Instantiate a WASM guest and call its `run` export.
    ///
    /// Deny-by-default: only the capabilities named in `--grant` are authorized.
    Run {
        /// Path to a WASM component.
        module: PathBuf,
        /// Comma-separated grants: dom, storage, network, media.
        #[arg(long, default_value = "")]
        grant: String,
        /// App identity used for capability grants and audit logs.
        #[arg(long, default_value = "app")]
        app: String,
        /// Bypass the capability broker entirely. Trusted local development only.
        #[arg(long)]
        unrestricted: bool,
    },
    /// Render an HTML document to a PNG (headless, no GPU needed).
    Render {
        /// HTML file to render.
        input: PathBuf,
        /// Extra stylesheet applied after the document's own `<style>` blocks.
        #[arg(long)]
        css: Option<PathBuf>,
        /// Output PNG path (defaults to the input name with a .png extension).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 800)]
        width: u32,
        #[arg(long, default_value_t = 600)]
        height: u32,
    },
    /// Migrate a JS/HTML app through stages S1–S5.
    Migrate {
        /// HTML document to migrate.
        input: PathBuf,
        /// Optional JS/TS source for discovery and type inference.
        #[arg(long)]
        js: Option<PathBuf>,
        /// App name (defaults to the input file stem).
        #[arg(long)]
        name: Option<String>,
        /// Print the full migration report.
        #[arg(long)]
        report: bool,
        /// Write the generated guest crate to this directory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Exit non-zero if the migration has any warnings.
        #[arg(long)]
        strict: bool,
    },
    /// Build a guest crate for wasm32-unknown-unknown.
    Build {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Create a new static starter app.
    New {
        /// Directory to create.
        name: PathBuf,
    },
    /// Check the toolchain needed by the other commands.
    Doctor,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<ExitCode, CliError> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Run {
            module,
            grant,
            app,
            unrestricted,
        } => {
            let bytes = std::fs::read(&module)?;
            let grants = parse_grants(&grant)?;
            run_module(&bytes, &app, &grants, unrestricted)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Render {
            input,
            css,
            output,
            width,
            height,
        } => {
            let html = std::fs::read_to_string(&input)?;
            let extra = match css {
                Some(path) => std::fs::read_to_string(path)?,
                None => String::new(),
            };
            let png = render_html(&html, &extra, width, height)?;
            let out = output.unwrap_or_else(|| input.with_extension("png"));
            std::fs::write(&out, png)?;
            println!("wrote {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Migrate {
            input,
            js,
            name,
            report,
            out,
            strict,
        } => {
            let html = std::fs::read_to_string(&input)?;
            let js_src = match js {
                Some(path) => Some(std::fs::read_to_string(path)?),
                None => None,
            };
            let name = name.unwrap_or_else(|| {
                input
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "app".into())
            });
            if !report && out.is_none() {
                return Err(CliError(
                    "nothing to do: pass --report and/or --out <dir>".into(),
                ));
            }
            let (migration, guest) = migrate_html(&name, &html, js_src.as_deref());
            if report {
                print!("{}", migration_report(&migration));
            }
            if let Some(dir) = out {
                let Some(guest) = guest else {
                    return Err(CliError(
                        "no supported pattern matched; nothing to write (see the report)".into(),
                    ));
                };
                write_guest_crate(&dir, &guest)?;
                println!("wrote guest crate to {}", dir.display());
            }
            if strict && !migration_warnings(&migration).is_empty() {
                return Ok(ExitCode::FAILURE);
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Build { dir } => {
            build_guest(&dir)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::New { name } => {
            new_project(&name)?;
            println!("created {}; try `helix render {}/index.html`", name.display(), name.display());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Doctor => {
            let checks = doctor_checks();
            for c in &checks {
                let tag = match (c.ok, c.required) {
                    (true, _) => "ok     ",
                    (false, true) => "MISSING",
                    (false, false) => "warn   ",
                };
                println!("[{tag}] {:<14} {}", c.name, c.detail);
                if !c.ok {
                    println!("          hint: {}", c.hint);
                }
            }
            Ok(if all_required_ok(&checks) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}
