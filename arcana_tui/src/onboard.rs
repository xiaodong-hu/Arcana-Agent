use std::io::{self, Write};
use std::path::PathBuf;

use crate::cli::OnboardArgs;
use crate::config::{Config, ProviderEntry};

/// Run the onboarding wizard.
pub async fn run(args: OnboardArgs) -> Result<(), Box<dyn std::error::Error>> {
    let home = dirs::home_dir().ok_or("cannot find home directory")?;
    let arcana_home = home.join(".arcana");

    println!();
    println!("  Welcome to Arcana — The Arcane Agent");
    println!("  Let's set up your environment.");
    println!();

    let mut config = Config::default();

    // Step 1: Provider selection
    let provider = if let Some(ref p) = args.provider {
        p.clone()
    } else if args.non_interactive {
        "deepseek".into()
    } else {
        select_provider()?
    };

    // Step 2: API Key
    let api_key = resolve_api_key(&provider, args.non_interactive)?;

    // Step 3: Model selection
    let model = if let Some(ref m) = args.model {
        m.clone()
    } else if args.non_interactive {
        default_model_for_provider(&provider)
    } else {
        select_model(&provider)?
    };

    // Configure the provider
    match provider.as_str() {
        "deepseek" => {
            config.providers.deepseek = ProviderEntry {
                api_key: api_key.unwrap_or_default(),
                base_url: "https://api.deepseek.com/beta".into(),
                models: vec!["deepseek-v4-pro".into(), "deepseek-v4-flash".into()],
            };
        }
        "openai" => {
            config.providers.openai = ProviderEntry {
                api_key: api_key.unwrap_or_default(),
                base_url: "https://api.openai.com/v1".into(),
                models: vec!["gpt-4o".into(), "o3".into()],
            };
        }
        "anthropic" => {
            config.providers.anthropic = ProviderEntry {
                api_key: api_key.unwrap_or_default(),
                base_url: "https://api.anthropic.com".into(),
                models: vec!["claude-sonnet-4-20250514".into()],
            };
        }
        _ => {}
    }

    config.agents.main.provider = provider.clone();
    config.agents.main.model = model.clone();
    config.agents.query.provider = provider.clone();
    config.agents.query.model = model.clone();
    config.agents.sub.provider = provider.clone();
    config.agents.sub.model = model;

    // Step 4: Create directory structure
    println!("  Creating ~/.arcana/ ...");
    create_global_directory(&arcana_home, &config)?;

    println!();
    println!("  ✓ Setup complete!");
    println!("  Run `arcana` in any project directory to start.");
    println!();

    Ok(())
}

fn select_provider() -> Result<String, Box<dyn std::error::Error>> {
    println!("  Step 1/4: Model Provider");
    println!();
    println!("  Which provider do you want to use?");
    println!();
    println!("    1. DeepSeek (deepseek-v4-pro, deepseek-v4-flash)");
    println!("    2. OpenAI (gpt-4o, o3)");
    println!("    3. Anthropic (claude-sonnet-4)");
    println!("    4. OpenRouter (any model)");
    println!("    5. Local (Ollama, vLLM, SGLang)");
    println!();
    print!("  Select [1-5, default=1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice = input.trim();

    Ok(match choice {
        "2" => "openai".into(),
        "3" => "anthropic".into(),
        "4" => "openrouter".into(),
        "5" => "local".into(),
        _ => "deepseek".into(),
    })
}

fn resolve_api_key(
    provider: &str,
    non_interactive: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let env_var = match provider {
        "deepseek" => "DEEPSEEK_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => return Ok(None),
    };

    // Check environment variable first
    if let Ok(_key) = std::env::var(env_var) {
        println!("  ✓ Found {} in environment.", env_var);
        return Ok(None); // Don't store it — env var is source of truth
    }

    if non_interactive {
        eprintln!("  ⚠ No API key found in {}. Set it before using Arcana.", env_var);
        return Ok(None);
    }

    // Interactive prompt
    println!();
    println!("  Step 2/4: API Key");
    println!();
    println!("  No {} found in environment.", env_var);
    print!("  Enter API key (or press Enter to skip): ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let key = input.trim().to_string();

    if key.is_empty() {
        println!("  Skipped. Set {} in your shell profile later.", env_var);
        Ok(None)
    } else {
        Ok(Some(key))
    }
}

fn select_model(provider: &str) -> Result<String, Box<dyn std::error::Error>> {
    println!();
    println!("  Step 3/4: Default Model");
    println!();

    let models = match provider {
        "deepseek" => vec!["deepseek-v4-pro", "deepseek-v4-flash"],
        "openai" => vec!["gpt-4o", "o3"],
        "anthropic" => vec!["claude-sonnet-4-20250514"],
        _ => vec!["auto"],
    };

    for (i, model) in models.iter().enumerate() {
        let marker = if i == 0 { " (recommended)" } else { "" };
        println!("    {}. {}{}", i + 1, model, marker);
    }

    print!("  Select [default=1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().unwrap_or(1);
    let idx = (choice - 1).min(models.len() - 1);

    Ok(models[idx].to_string())
}

fn default_model_for_provider(provider: &str) -> String {
    match provider {
        "deepseek" => "deepseek-v4-pro".into(),
        "openai" => "gpt-4o".into(),
        "anthropic" => "claude-sonnet-4-20250514".into(),
        _ => "auto".into(),
    }
}

fn create_global_directory(
    arcana_home: &PathBuf,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create directory structure
    std::fs::create_dir_all(arcana_home)?;
    std::fs::create_dir_all(arcana_home.join("models"))?;
    std::fs::create_dir_all(arcana_home.join("skills").join("system"))?;
    std::fs::create_dir_all(arcana_home.join("skills").join("user"))?;

    // Write config.toml
    let config_content = toml::to_string_pretty(config)?;
    std::fs::write(arcana_home.join("config.toml"), config_content)?;

    // Write default SOUL.md
    let soul_md = r#"# SOUL.md — Arcana Agent Personality

## Tone
- Direct, concise, no filler
- Match user's technical level

## Preferences
- Show reasoning before conclusions
- Use code examples over prose explanations

## Constraints
- Never apologize unnecessarily
- Do not repeat information already stated
"#;
    std::fs::write(arcana_home.join("SOUL.md"), soul_md)?;

    // Write empty USER.md
    let user_md = r#"# USER.md — User Portrait

## Background
(Populated automatically from interactions)

## Preferences
(Populated automatically from interactions)

## Communication Style
(Populated automatically from interactions)
"#;
    std::fs::write(arcana_home.join("USER.md"), user_md)?;

    // Write empty tools.toml
    std::fs::write(arcana_home.join("tools.toml"), "# Runtime-registered tools\n")?;

    println!("  ✓ Created ~/.arcana/config.toml");
    println!("  ✓ Created ~/.arcana/SOUL.md");
    println!("  ✓ Created ~/.arcana/USER.md");

    Ok(())
}
