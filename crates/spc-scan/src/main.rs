use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "spc-scan", version = spc_scan::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the frozen SPc vectors.
    Vectors,
    /// List a block's SPc carriers (implementation lands in WP2).
    Find,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Vectors) => println!("spc-scan vectors: not implemented yet"),
        Some(Command::Find) => println!("spc-scan find: not implemented yet"),
        None => println!("spc-scan {} (use --help)", spc_scan::VERSION),
    }
}
