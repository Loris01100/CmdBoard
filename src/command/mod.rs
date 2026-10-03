/// Every user action. Keys (and from step 5 the `:` command line and aliases) are
/// translated into a `Command`, then run by `App::execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Launch { app: String },
    Quit,
}
