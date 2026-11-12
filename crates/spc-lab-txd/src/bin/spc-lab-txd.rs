use std::io;
use std::path::PathBuf;
fn main() {
    if let Err(error) = run() {
        eprintln!("spc-lab-txd: {error}");
        std::process::exit(1);
    }
}
fn run() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 || args[0] != "--config" || args[2] != "--incarnation" {
        return Err(spc_lab_txd::control::invalid(
            "usage: spc-lab-txd --config PATH --incarnation ID",
        ));
    }
    let config = spc_lab_txd::actor_config::ActorConfig::load(&PathBuf::from(&args[1]))?;
    let (tx, rx) = std::sync::mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match spc_lab_txd::control::read_message(&mut input) {
                Ok(Some(message)) => {
                    if tx.send(Ok(message)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    spc_lab_txd::actor::Actor::new(config, args[3].clone(), rx, io::stdout().lock())?.run()
}
