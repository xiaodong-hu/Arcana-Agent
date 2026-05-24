mod authority;
mod prompt;
mod record;
mod server;
mod types;

use std::env;
use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Subcommand: `authority_and_recording auth instruction [project_root]`
    if args.len() >= 3 && args[1] == "auth" && args[2] == "instruction" {
        match prompt::load_or_create_instruction() {
            Ok(content) => print!("{}", content),
            Err(e) => { eprintln!("[Arcana] Error: {}", e); process::exit(1); }
        }
        return;
    }

    // Compatibility subcommand: print the full injected prompt.
    if args.len() >= 3 && args[1] == "auth" && args[2] == "prompt" {
        let root = args.get(3).map(PathBuf::from)
            .unwrap_or_else(|| env::current_dir().expect("cannot get cwd"));
        match run_prompt(&root) {
            Ok(content) => print!("{}", content),
            Err(e) => { eprintln!("[Arcana] Error: {}", e); process::exit(1); }
        }
        return;
    }

    // Default: run the server
    let project_root = args.get(1).map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("cannot get cwd"));

    if !project_root.is_dir() {
        eprintln!("[Arcana] Error: {:?} is not a directory", project_root);
        process::exit(1);
    }

    eprintln!("[Arcana] Authority & Record starting for {:?}", project_root);

    let mut srv = match server::Server::new(project_root) {
        Ok(s) => s,
        Err(e) => { eprintln!("[Arcana] Failed to start: {}", e); process::exit(1); }
    };

    if let Err(e) = srv.run() {
        eprintln!("[Arcana] Server error: {}", e);
        process::exit(1);
    }
}

/// Generate and print the authorized prompt without starting the server.
fn run_prompt(project_root: &PathBuf) -> std::io::Result<String> {
    let auth = authority::Authority::load(project_root.clone())?;
    let content = prompt::generate_prompt(&auth)?;
    // Also write to .arcana/authorized_prompt.md
    let prompt_path = project_root.join(".arcana/authorized_prompt.md");
    std::fs::create_dir_all(prompt_path.parent().unwrap())?;
    std::fs::write(&prompt_path, &content)?;
    Ok(content)
}
