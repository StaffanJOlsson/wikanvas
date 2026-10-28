//! Configuration of §7.6: flags, environment and the profile file (owner: W8)

use crate::common::Sandbox;

#[test]
fn token_stdin_with_token_file_exit_2() {
    let sandbox = Sandbox::new();
    for args in [
        &["--token-stdin", "--token-file", "t", "exit-codes"][..],
        &["exit-codes", "--token-file", "t", "--token-stdin"][..],
    ] {
        sandbox.quire().args(args).assert().code(2).stdout("");
    }
}
