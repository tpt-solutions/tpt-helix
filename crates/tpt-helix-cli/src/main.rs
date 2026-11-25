//! `helix` binary: argument parsing and output. Behavior lives in `tpt_helix_cli`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpt_helix_runtime::manifest;
use tpt_helix_cli::{
    CliError, GuestTemplate, all_required_ok, build_guest, doctor_checks, migrate_html,
    CompileCheck, confidence_report, diff_report, manifest_report, migration_report, migration_warnings, new_guest_project,
    new_project, parse_grants, read_manifest, render_html, run_module, write_guest_crate,
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
        /// Print the confidence block: pattern, equivalence, warnings, and compile status.
        #[arg(long)]
        confidence: bool,
        /// Build the written crate for wasm32 (needs --out). Implies --confidence,
        /// and exits non-zero if the build fails.
        #[arg(long)]
        check: bool,
    },
    /// Build a guest crate for wasm32-unknown-unknown.
    Build {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Create a new starter app or guest crate.
    New {
        /// Directory to create.
        name: PathBuf,
        /// `static` (an HTML page), or a guest template: `basic`, `network`,
        /// or `storage`. Guest templates ship a `capabilities` grant list.
        #[arg(long, default_value = "static")]
        template: String,
    },
    /// Infer the capabilities a component needs from its imports (spec §10.1).
    Manifest {
        /// Component to inspect.
        module: PathBuf,
        /// Older component to compare against. Exits 2 when the new one adds a grant.
        #[arg(long)]
        diff: Option<PathBuf>,
    },
    /// Generate an Ed25519 signing key. Writes the 32-byte seed as hex to `out`
    /// and prints the public key. Keep the seed file private.
    Keygen {
        /// File to write the seed to. Must not exist.
        out: PathBuf,
    },
    /// Sign a directory as a content-addressed bundle (spec §10.4).
    Bundle {
        /// Directory to sign. Its files are hashed, and `bundle.json` is written into it.
        dir: PathBuf,
        /// File holding the signing seed (hex), from `helix keygen`.
        #[arg(long)]
        key: PathBuf,
        /// App name recorded in the manifest (defaults to the directory name).
        #[arg(long)]
        app: Option<String>,
        #[arg(long, default_value = "0.1.0")]
        version: String,
    },
    /// Verify a signed bundle against a trusted public key (spec §10.4).
    Verify {
        /// Bundle directory to check.
        dir: PathBuf,
        /// File holding the trusted public key (hex). The key the bundle embeds is
        /// never trusted on its own.
        #[arg(long)]
        trust: PathBuf,
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
            confidence,
            check,
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
            if check && out.is_none() {
                return Err(CliError("--check builds the generated crate; pass --out <dir>".into()));
            }
            let mut compile = CompileCheck::NotRun;
            if let Some(dir) = out {
                let Some(guest) = guest else {
                    return Err(CliError(
                        "no supported pattern matched; nothing to write (see the report)".into(),
                    ));
                };
                write_guest_crate(&dir, &guest)?;
                println!("wrote guest crate to {}", dir.display());
                if check {
                    compile = match build_guest(&dir) {
                        Ok(()) => CompileCheck::Passed,
                        Err(e) => {
                            eprintln!("{e}");
                            CompileCheck::Failed
                        }
                    };
                }
            }
            if confidence || check {
                print!("{}", confidence_report(&migration, compile));
            }
            if compile == CompileCheck::Failed {
                return Ok(ExitCode::FAILURE);
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
        Cmd::New { name, template } => {
            if template == "static" {
                new_project(&name)?;
                println!(
                    "created {}; try `helix render {}/index.html`",
                    name.display(),
                    name.display()
                );
            } else {
                let guest_template = GuestTemplate::parse(&template)?;
                new_guest_project(&name, guest_template)?;
                println!(
                    "created {}; build with `helix build {}`, then run it with `helix run <module.wasm> --grant \"$(cat {}/capabilities)\"`",
                    name.display(),
                    name.display(),
                    name.display()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Manifest { module, diff } => {
            let new = read_manifest(&module)?;
            match diff {
                None => {
                    print!("{}", manifest_report(&new));
                    Ok(ExitCode::SUCCESS)
                }
                Some(old_path) => {
                    let old = read_manifest(&old_path)?;
                    let d = manifest::diff(&old, &new);
                    print!("{}", diff_report(&d));
                    Ok(if d.requests_new_grants() {
                        ExitCode::from(2)
                    } else {
                        ExitCode::SUCCESS
                    })
                }
            }
        }
        Cmd::Keygen { out } => {
            if out.exists() {
                return Err(CliError(format!(
                    "{} already exists; choose another path",
                    out.display()
                )));
            }
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed)
                .map_err(|e| CliError(format!("no OS randomness: {e}")))?;
            let seed_hex = tpt_helix_cli::bundle::to_hex(&seed);
            std::fs::write(&out, format!("{seed_hex}
"))?;
            let public = tpt_helix_cli::bundle::public_key_hex(&seed_hex)?;
            println!("wrote seed to {} (keep it private)", out.display());
            println!("public key: {public}");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Bundle { dir, key, app, version } => {
            let seed = std::fs::read_to_string(&key)?;
            let app = app.unwrap_or_else(|| {
                dir.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "app".to_string())
            });
            let id = tpt_helix_cli::bundle::sign_bundle(&dir, &app, &version, &seed)?;
            println!("signed {app} {version}; content id {id}");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Verify { dir, trust } => {
            let trusted = std::fs::read_to_string(&trust)?;
            let v = tpt_helix_cli::bundle::verify_bundle(&dir, &trusted)?;
            println!("verified {} {}; content id {}", v.app, v.version, v.content_id);
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
