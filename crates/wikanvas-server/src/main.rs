//! The quire-server binary: serve, healthcheck and admin (owner: W2)

fn main() -> std::process::ExitCode {
    quire_server::cli::main()
}
