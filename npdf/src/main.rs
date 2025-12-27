mod commands;
mod common;

use clap::{
    Parser, Subcommand,
    builder::{
        Styles,
        styling::{AnsiColor, Effects},
    },
};
use color_print::cprintln;
use commands::{
    ExportArgs, ListArgs, RecropArgs, UnwatermarkArgs, export, list, recrop, unwatermark,
};
use tiny_poppler::PdfPasswords;

fn main() {
    let cli = Cli::parse();
    if let Err(err) = execute(cli) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    let Cli {
        command,
        user_password,
        owner_password,
    } = cli;

    let passwords = build_passwords(owner_password, user_password);

    match command {
        Commands::List(args) => list::run(args, passwords.as_ref()),
        Commands::Export(args) => export::run(args, passwords.as_ref()),
        Commands::Unwatermark(args) => unwatermark::run(args, passwords.as_ref()),
        Commands::Recrop(args) => recrop::run(args, passwords.as_ref()),
        Commands::Version => cmd_show_version_info(),
    }
}

#[derive(Parser)]
#[command(name = "npdf")]
#[command(author, version, about, long_about = None, styles = cli_styles())]
#[command(propagate_version = true, disable_help_subcommand = true)]
struct Cli {
    /// User password for encrypted PDFs.
    #[arg(
        long = "password",
        alias = "user-password",
        global = true,
        value_name = "PASSWORD"
    )]
    user_password: Option<String>,
    /// Owner password for encrypted PDFs.
    #[arg(long = "owner-password", global = true, value_name = "PASSWORD")]
    owner_password: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List images embedded in the PDF.
    List(ListArgs),
    /// Export/extract pages from the PDF to PNG or JPEG files.
    Export(ExportArgs),
    /// Remove watermarks from the PDF.
    Unwatermark(UnwatermarkArgs),
    /// Recrop pages in the PDF based on specified box.
    Recrop(RecropArgs),
    /// Get version information.
    Version,
}

fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Magenta.on_default() | Effects::BOLD | Effects::UNDERLINE)
        .literal(AnsiColor::Blue.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::BrightCyan.on_default())
}

fn build_passwords(
    owner_password: Option<String>,
    user_password: Option<String>,
) -> Option<PdfPasswords> {
    if owner_password.is_none() && user_password.is_none() {
        None
    } else {
        Some(PdfPasswords::new(owner_password, user_password))
    }
}

fn cmd_show_version_info() -> Result<(), String> {
    let poppler_version = tiny_poppler::get_version();
    cprintln!("<s>npdf</s> version: {}", env!("CARGO_PKG_VERSION"));
    if let Some(sha_commit) = poppler_version.git_sha() {
        cprintln!(
            "<s>poppler</s> version: {}+g{}",
            poppler_version.version_string(),
            sha_commit.chars().take(7).collect::<String>()
        );
    } else {
        cprintln!(
            "<s>poppler</s> version: {}",
            poppler_version.version_string()
        );
    }

    let jxl_version = simple_jpegli_enc::get_version();
    if let Some(sha_commit) = jxl_version.git_sha() {
        cprintln!(
            "<s>libjxl</s> version: {}+g{} (jpegli backend, compat {})",
            jxl_version.version_string(),
            sha_commit.chars().take(7).collect::<String>(),
            jxl_version.lib_version()
        );
    } else {
        cprintln!(
            "<s>libjxl</s> version: {} (jpegli backend, compat {})",
            jxl_version.version_string(),
            jxl_version.lib_version()
        );
    }

    Ok(())
}
