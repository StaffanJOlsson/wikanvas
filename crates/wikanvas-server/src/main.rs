//! The wikanvas-server binary: serve, healthcheck and admin (owner: W2)

fn main() -> std::process::ExitCode {
    wikanvas_server::cli::main()
}
