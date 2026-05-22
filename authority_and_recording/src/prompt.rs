use crate::authority::Authority;

/// Generate the authorized prompt markdown that tells the LLM what tools and
/// permissions are available, and how to invoke them via the authority socket.
pub fn generate_prompt(authority: &Authority) -> String {
    let mut out = String::new();

    out.push_str("# Arcana Authority — Available Tools & Permissions\n\n");
    out.push_str("You are operating under the Arcana authority system. ALL file operations, \
                  command execution, and web access MUST go through the authority IPC socket.\n\n");
    out.push_str("## IPC Protocol\n\n");
    out.push_str("Send one JSON object per line to the unix socket at `.arcana/authority.sock`.\n");
    out.push_str("Each request gets one JSON response line.\n\n");

    // Tools section
    out.push_str("## Available Operations\n\n");
    out.push_str("### File Read\n");
    out.push_str("```json\n{\"op\": \"read\", \"path\": \"<relative_path>\"}\n```\n");
    out.push_str("Response: `{\"status\": \"content\", \"data\": \"<base64>\"}` or `{\"status\": \"denied\", \"reason\": \"...\"}`\n\n");

    out.push_str("### File Write\n");
    out.push_str("```json\n{\"op\": \"write\", \"path\": \"<relative_path>\", \"content\": \"<base64>\"}\n```\n");
    out.push_str("Response: `{\"status\": \"ok\"}` or `{\"status\": \"denied\", \"reason\": \"...\"}`\n\n");

    out.push_str("### File Delete\n");
    out.push_str("```json\n{\"op\": \"delete\", \"path\": \"<relative_path>\"}\n```\n\n");

    out.push_str("### File Rename\n");
    out.push_str("```json\n{\"op\": \"rename\", \"src\": \"<path>\", \"dst\": \"<path>\"}\n```\n\n");

    out.push_str("### Query Permission\n");
    out.push_str("```json\n{\"op\": \"query\", \"path\": \"<relative_path>\"}\n```\n");
    out.push_str("Response: `{\"status\": \"permission\", \"level\": \"none|read|write\"}`\n\n");

    out.push_str("### Execute Command\n");
    out.push_str("```json\n{\"op\": \"exec\", \"cmd\": \"<command>\", \"args\": [\"arg1\", \"arg2\"]}\n```\n");
    out.push_str("Response: `{\"status\": \"exec_result\", \"stdout\": \"...\", \"stderr\": \"...\", \"code\": 0}`\n\n");

    out.push_str("### Web Fetch\n");
    out.push_str("```json\n{\"op\": \"fetch\", \"url\": \"<url>\", \"tag\": null}\n```\n");
    out.push_str("Response: `{\"status\": \"fetched\", \"path\": \"<cache_path>\", \"bytes\": N}`\n\n");

    out.push_str("### Register Tool\n");
    out.push_str("```json\n{\"op\": \"register_tool\", \"name\": \"<name>\", \"path\": \"<binary>\", \"args\": [], \"description\": \"...\"}\n```\n\n");

    // Current permissions
    out.push_str("## Current Permissions\n\n");

    let rules = authority.access_rules();
    out.push_str("### Write Access\n");
    if rules.allow_write.is_empty() {
        out.push_str("- No paths pre-approved for write (will prompt user)\n");
    } else {
        for p in &rules.allow_write {
            out.push_str(&format!("- ✓ `{}`\n", p));
        }
    }
    if !rules.deny_write.is_empty() {
        out.push_str("\nDenied (never writable):\n");
        for p in &rules.deny_write {
            out.push_str(&format!("- ✗ `{}`\n", p));
        }
    }
    out.push_str(&format!("\nDefault for unlisted paths: **{}**\n\n", rules.default));

    out.push_str("### Read Access\n");
    if rules.deny_read.is_empty() {
        out.push_str("- All files readable (no deny rules)\n");
    } else {
        out.push_str("Denied (invisible to you):\n");
        for p in &rules.deny_read {
            out.push_str(&format!("- ✗ `{}`\n", p));
        }
    }
    out.push('\n');

    // Web access
    let web = authority.web_config();
    out.push_str("### Web Access\n");
    if !web.allow_domains.is_empty() {
        out.push_str("Allowed domains:\n");
        for d in &web.allow_domains {
            out.push_str(&format!("- ✓ `{}`\n", d));
        }
    }
    if !web.deny_domains.is_empty() {
        out.push_str("Denied domains:\n");
        for d in &web.deny_domains {
            out.push_str(&format!("- ✗ `{}`\n", d));
        }
    }
    out.push_str(&format!("Default: **{}**\n\n", web.default));

    // Tools
    let tools = authority.tools_config();
    out.push_str("### Command Execution\n");
    if !tools.allow.is_empty() {
        out.push_str("Pre-approved commands:\n");
        for t in &tools.allow {
            out.push_str(&format!("- ✓ `{}`\n", t));
        }
    }
    if !tools.deny.is_empty() {
        out.push_str("Denied commands:\n");
        for t in &tools.deny {
            out.push_str(&format!("- ✗ `{}`\n", t));
        }
    }
    out.push_str(&format!("Runtime tool registration: **{}**\n", if tools.allow_runtime_registration { "enabled" } else { "disabled" }));
    out.push_str(&format!("Default for unlisted commands: **{}**\n\n", tools.default));

    // Important notes
    out.push_str("## Important Notes\n\n");
    out.push_str("- All file writes are **recorded** and recoverable. Every mutation is logged.\n");
    out.push_str("- If a request is denied, do NOT retry — inform the user.\n");
    out.push_str("- Operations with `default = \"prompt\"` will ask the human for approval. Wait for the response.\n");
    out.push_str("- File content in `write` requests must be base64-encoded.\n");
    out.push_str("- File content in `read` responses is base64-encoded.\n");

    out
}
