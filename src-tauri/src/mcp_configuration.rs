//! Configuration and managed AGENTS rules ported from the account tool.
use crate::provider_config::write_atomic;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table};

const IMAGE_AGENTS_BEGIN: &str = "<!-- ChatGPT账号工具:generate-image:start -->";
const IMAGE_AGENTS_END: &str = "<!-- ChatGPT账号工具:generate-image:end -->";
const IMAGE_AGENTS_RULES: &str = "## 生图与改图\n\n生成图片必须调用 Generate image MCP 的 `generate_image` 工具。\n修改图片必须调用 Generate image MCP 的 `edit_image` 工具。\n不要使用内置 `imagegen` 技能。若工具不可用，告知用户在 ChatGPT中转工具中点击“使用生图”或“修复生图”并重启 ChatGPT；不要自动改用其他生图工具。";

struct FileSnapshot {
    existed: bool,
    bytes: Vec<u8>,
}

pub fn provider_is_eligible(config: &[u8]) -> Result<bool, String> {
    let text = std::str::from_utf8(config).map_err(|error| error.to_string())?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let provider = document
        .get("model_provider")
        .and_then(Item::as_str)
        .unwrap_or("openai");
    if provider.eq_ignore_ascii_case("openai") {
        return Ok(false);
    }
    Ok(document
        .get("model_providers")
        .and_then(Item::as_table_like)
        .and_then(|providers| providers.get(provider))
        .and_then(Item::as_table_like)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .is_some_and(|url| url.to_ascii_lowercase().contains("ai.711ev.com")))
}

pub fn remove_image_configuration(config: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(config).map_err(|error| error.to_string())?;
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let mut comments = String::new();
    if let Some(servers) = document.get_mut("mcp_servers") {
        let servers = servers
            .as_table_like_mut()
            .ok_or("mcp_servers 不是有效配置表")?;
        if let Some(removed) = servers.remove("generate_image") {
            preserve_table_comments(&removed, &mut comments);
        }
        if servers.is_empty() {
            if let Some(removed) = document.remove("mcp_servers") {
                preserve_table_comments(&removed, &mut comments);
            }
        }
    }
    if !comments.is_empty() {
        comments.push_str(document.trailing().as_str().unwrap_or_default());
        document.set_trailing(format!("\n{comments}"));
    }
    Ok(document.to_string().into_bytes())
}

fn preserve_table_comments(item: &Item, output: &mut String) {
    let decor = item
        .as_table()
        .map(Table::decor)
        .or_else(|| item.as_value().map(|value| value.decor()));
    if let Some(decor) = decor {
        for raw in [decor.prefix(), decor.suffix()].into_iter().flatten() {
            let text = raw.as_str().unwrap_or_default();
            if text.contains('#') {
                output.push_str(text);
                if !text.ends_with('\n') {
                    output.push('\n');
                }
            }
        }
    }
}

pub fn remove_image_agents_rules(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let after = transform_image_agents_rules(bytes, false)?;
    let text = std::str::from_utf8(&after).map_err(|error| error.to_string())?;
    if text.trim_start_matches('\u{feff}').trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(after))
    }
}

pub fn configure(home: &Path, executable: &Path) -> Result<(), String> {
    let config_path = home.join("config.toml");
    let agents_path = home.join("AGENTS.md");
    let config = fs::read_to_string(&config_path)
        .map_err(|error| format!("无法读取 Codex 配置：{error}"))?;
    let mut document = config
        .parse::<DocumentMut>()
        .map_err(|error| format!("Codex 配置格式错误：{error}"))?;
    let servers = ensure_table(document.as_table_mut(), "mcp_servers")?;
    let image = ensure_table(servers, "generate_image")?;
    set_string(image, "command", &executable.to_string_lossy());
    image.insert("args", value(Array::new()));
    image.insert("startup_timeout_sec", value(30));
    image.insert("tool_timeout_sec", value(120));
    let agents = ensure_image_agents_rules(&read_file_snapshot(&agents_path)?.bytes)?;
    commit_codex_configuration_files(
        &config_path,
        document.to_string().as_bytes(),
        &agents_path,
        Some(&agents),
    )
}

pub fn inspect(home: &Path, executable: &Path) -> Result<(bool, bool), String> {
    let config = fs::read_to_string(home.join("config.toml"))
        .map_err(|error| format!("无法读取 Codex 配置：{error}"))?;
    let document = config
        .parse::<DocumentMut>()
        .map_err(|error| format!("Codex 配置格式错误：{error}"))?;
    let command = document
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .and_then(|servers| servers.get("generate_image"))
        .and_then(Item::as_table_like)
        .and_then(|image| image.get("command"))
        .and_then(Item::as_str);
    let registered = command.is_some_and(|command| {
        match (fs::canonicalize(command), fs::canonicalize(executable)) {
            (Ok(left), Ok(right)) => left == right,
            _ => Path::new(command) == executable,
        }
    });
    if !registered {
        return Ok((false, false));
    }
    let before = read_file_snapshot(&home.join("AGENTS.md"))?;
    let configured = before.existed && ensure_image_agents_rules(&before.bytes)? == before.bytes;
    Ok((true, configured))
}

fn ensure_image_agents_rules(bytes: &[u8]) -> Result<Vec<u8>, String> {
    transform_image_agents_rules(bytes, true)
}

fn transform_image_agents_rules(bytes: &[u8], ensure: bool) -> Result<Vec<u8>, String> {
    const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];
    let (has_bom, payload) = if let Some(payload) = bytes.strip_prefix(UTF8_BOM) {
        (true, payload)
    } else {
        (false, bytes)
    };
    let text = std::str::from_utf8(payload)
        .map_err(|_| "AGENTS.md 不是有效的 UTF-8 文本，无法安全更新生图规则".to_string())?;
    let newline = agents_newline(text);
    let ranges = image_agents_block_ranges(text)?;
    let updated = if ensure {
        let canonical = image_agents_block(newline);
        if ranges.is_empty() {
            append_image_agents_block(text, &canonical, newline)
        } else {
            replace_image_agents_blocks(text, &ranges, Some(&canonical), newline)
        }
    } else if ranges.is_empty() {
        text.to_string()
    } else {
        replace_image_agents_blocks(text, &ranges, None, newline)
    };

    let mut output = Vec::with_capacity(updated.len() + usize::from(has_bom) * UTF8_BOM.len());
    if has_bom {
        output.extend_from_slice(UTF8_BOM);
    }
    output.extend_from_slice(updated.as_bytes());
    Ok(output)
}

fn agents_newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

fn image_agents_block(newline: &str) -> String {
    format!(
        "{IMAGE_AGENTS_BEGIN}{newline}{}{newline}{IMAGE_AGENTS_END}",
        IMAGE_AGENTS_RULES.replace('\n', newline)
    )
}

fn append_image_agents_block(text: &str, block: &str, newline: &str) -> String {
    let mut output = String::with_capacity(text.len() + block.len() + newline.len() * 2);
    output.push_str(text);
    if !text.is_empty() {
        if text.ends_with(&format!("{newline}{newline}")) {
            // Existing Markdown already has a paragraph boundary.
        } else if text.ends_with(newline) {
            output.push_str(newline);
        } else {
            output.push_str(newline);
            output.push_str(newline);
        }
    }
    output.push_str(block);
    output.push_str(newline);
    output
}

fn image_agents_block_ranges(text: &str) -> Result<Vec<(usize, usize)>, String> {
    let mut ranges = Vec::new();
    let mut open = None;
    let mut offset = 0;
    let mut begin_lines = 0;
    let mut end_lines = 0;

    for line in text.split_inclusive('\n') {
        let end = offset + line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        let trimmed = content.trim_matches(|character| character == ' ' || character == '\t');
        if trimmed == IMAGE_AGENTS_BEGIN {
            begin_lines += 1;
            if open.replace(offset).is_some() {
                return Err(image_agents_marker_error());
            }
        } else if trimmed == IMAGE_AGENTS_END {
            end_lines += 1;
            let start = open.take().ok_or_else(image_agents_marker_error)?;
            ranges.push((start, end));
        }
        offset = end;
    }

    if open.is_some()
        || begin_lines != text.matches(IMAGE_AGENTS_BEGIN).count()
        || end_lines != text.matches(IMAGE_AGENTS_END).count()
    {
        return Err(image_agents_marker_error());
    }
    Ok(ranges)
}

fn image_agents_marker_error() -> String {
    "AGENTS.md 中的生图配置标记不完整，请先修复该文件".to_string()
}

fn replace_image_agents_blocks(
    text: &str,
    ranges: &[(usize, usize)],
    replacement: Option<&str>,
    newline: &str,
) -> String {
    let mut output = String::with_capacity(text.len() + replacement.map(str::len).unwrap_or(0));
    let mut cursor = 0;
    for (index, &(start, end)) in ranges.iter().enumerate() {
        output.push_str(&text[cursor..start]);
        if index == 0 {
            if let Some(replacement) = replacement {
                output.push_str(replacement);
                if text[start..end].ends_with('\n') {
                    output.push_str(newline);
                }
            }
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

fn commit_codex_configuration_files(
    config_path: &Path,
    config_after: &[u8],
    agents_path: &Path,
    agents_after: Option<&[u8]>,
) -> Result<(), String> {
    commit_codex_configuration_files_with(
        config_path,
        config_after,
        agents_path,
        agents_after,
        apply_file_update,
    )
}

fn commit_codex_configuration_files_with<F>(
    config_path: &Path,
    config_after: &[u8],
    agents_path: &Path,
    agents_after: Option<&[u8]>,
    mut apply: F,
) -> Result<(), String>
where
    F: FnMut(&Path, Option<&[u8]>) -> Result<(), String>,
{
    let config_before = read_file_snapshot(config_path)?;
    let agents_before = read_file_snapshot(agents_path)?;
    let commit = (|| {
        apply(config_path, Some(config_after))?;
        apply(agents_path, agents_after)
    })();
    if let Err(error) = commit {
        let agents_rollback = restore_file_snapshot(agents_path, &agents_before);
        let config_rollback = restore_file_snapshot(config_path, &config_before);
        let rollback_errors = [agents_rollback.err(), config_rollback.err()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if rollback_errors.is_empty() {
            return Err(error);
        }
        return Err(format!(
            "生图配置写入失败且回滚未完成：{}",
            rollback_errors.join("；")
        ));
    }
    Ok(())
}

fn read_file_snapshot(path: &Path) -> Result<FileSnapshot, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(FileSnapshot {
            existed: true,
            bytes,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileSnapshot {
            existed: false,
            bytes: Vec::new(),
        }),
        Err(error) => Err(format!("无法读取配置文件：{error}")),
    }
}

fn restore_file_snapshot(path: &Path, snapshot: &FileSnapshot) -> Result<(), String> {
    if snapshot.existed {
        apply_file_update(path, Some(&snapshot.bytes))
    } else {
        apply_file_update(path, None)
    }
}

fn apply_file_update(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    if let Some(bytes) = bytes {
        write_atomic(path, bytes).map_err(String::from)?;
        let written = fs::read(path).map_err(|error| error.to_string())?;
        if Sha256::digest(&written) != Sha256::digest(bytes) {
            return Err("生图配置写入校验失败".into());
        }
        Ok(())
    } else {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        match fs::metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err("配置文件删除校验失败".to_string()),
            Err(error) => Err(format!("无法校验配置文件删除结果：{error}")),
        }
    }
}

fn ensure_table<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, String> {
    if parent.get(key).is_none() {
        let mut table = Table::new();
        table.set_implicit(true);
        parent.insert(key, Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("config.toml 中 {key} 不是有效配置表"))
}

fn set_string(table: &mut Table, key: &str, text: &str) {
    if table.get(key).and_then(Item::as_str) == Some(text) {
        return;
    }
    let decor = table
        .get(key)
        .and_then(Item::as_value)
        .map(|current| current.decor().clone());
    let mut item = value(text);
    if let (Some(decor), Some(next)) = (decor, item.as_value_mut()) {
        *next.decor_mut() = decor;
    }
    table.insert(key, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_preserves_other_mcp_comments_and_auth_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let executable = home.join("image-mcp.exe");
        fs::write(&executable, b"mcp").unwrap();
        fs::write(
            home.join("config.toml"),
            "# keep\nmodel_provider = \"openai\"\n[mcp_servers.other]\ncommand = \"keep\"\n",
        )
        .unwrap();
        fs::write(home.join("auth.json"), b"do not change auth").unwrap();
        fs::write(home.join("AGENTS.md"), "# My rules\n").unwrap();
        configure(home, &executable).unwrap();
        let first = fs::read(home.join("config.toml")).unwrap();
        let agents = fs::read(home.join("AGENTS.md")).unwrap();
        configure(home, &executable).unwrap();
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), first);
        assert_eq!(fs::read(home.join("AGENTS.md")).unwrap(), agents);
        let config = String::from_utf8(first).unwrap();
        assert!(config.starts_with("# keep\n"));
        let document = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            document["mcp_servers"]["other"]["command"].as_str(),
            Some("keep")
        );
        assert_eq!(
            document["mcp_servers"]["generate_image"]["command"].as_str(),
            executable.to_str()
        );
        assert_eq!(
            document["mcp_servers"]["generate_image"]["args"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            document["mcp_servers"]["generate_image"]["startup_timeout_sec"].as_integer(),
            Some(30)
        );
        assert_eq!(
            document["mcp_servers"]["generate_image"]["tool_timeout_sec"].as_integer(),
            Some(120)
        );
        assert_eq!(
            fs::read(home.join("auth.json")).unwrap(),
            b"do not change auth"
        );
        assert_eq!(inspect(home, &executable).unwrap(), (true, true));
    }

    #[test]
    fn inspection_never_registers_mcp_or_restores_removed_rules() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let executable = home.join("image-mcp.exe");
        fs::write(&executable, b"mcp").unwrap();
        let config = b"model_provider = \"openai\"\n";
        fs::write(home.join("config.toml"), config).unwrap();
        assert_eq!(inspect(home, &executable).unwrap(), (false, false));
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config);
        assert!(!home.join("AGENTS.md").exists());
        configure(home, &executable).unwrap();
        fs::write(home.join("AGENTS.md"), "# Keep user rules\n").unwrap();
        assert_eq!(inspect(home, &executable).unwrap(), (true, false));
        assert_eq!(
            fs::read_to_string(home.join("AGENTS.md")).unwrap(),
            "# Keep user rules\n"
        );
        fs::remove_file(home.join("AGENTS.md")).unwrap();
        assert_eq!(inspect(home, &executable).unwrap(), (true, false));
        assert!(!home.join("AGENTS.md").exists());
    }

    #[test]
    fn eligibility_uses_only_the_applied_provider_and_literal_case_insensitive_match() {
        for (provider, url, expected) in [
            ("one", "https://ai.711ev.com/v1", true),
            ("two", "https://AI.711EV.COM/other", true),
            ("custom", "https://proxy.example/ai.711ev.com", true),
            ("one", "https://example.com/v1", false),
            ("one", "", false),
            ("openai", "https://ai.711ev.com/v1", false),
            ("OpenAI", "https://ai.711ev.com/v1", false),
        ] {
            let config = format!("model_provider = '{provider}'\n[model_providers.{provider}]\nbase_url = '{url}'\n[model_providers.unselected]\nbase_url = 'https://ai.711ev.com/v1'\n");
            assert_eq!(provider_is_eligible(config.as_bytes()).unwrap(), expected);
        }
        assert!(!provider_is_eligible(b"model_provider = 'missing'\n").unwrap());
        assert!(!provider_is_eligible(b"").unwrap());
        assert!(provider_is_eligible(b"not toml").is_err());
    }

    #[test]
    fn cleanup_preserves_other_servers_and_removes_an_empty_server_table() {
        let config = b"# keep\n[mcp_servers.generate_image]\ncommand = 'image'\n[mcp_servers.other]\ncommand = 'other'\n";
        let after = remove_image_configuration(config).unwrap();
        let text = std::str::from_utf8(&after).unwrap();
        assert!(text.contains("# keep\n"));
        let document = text.parse::<DocumentMut>().unwrap();
        assert!(document["mcp_servers"].get("generate_image").is_none());
        assert_eq!(
            document["mcp_servers"]["other"]["command"].as_str(),
            Some("other")
        );
        assert_eq!(remove_image_configuration(&after).unwrap(), after);
        for config in [
            "[mcp_servers.generate_image]\ncommand = 'image'\n",
            "mcp_servers = { generate_image = { command = 'image' } }\n",
        ] {
            let after = remove_image_configuration(config.as_bytes()).unwrap();
            assert!(!std::str::from_utf8(&after).unwrap().contains("mcp_servers"));
        }
    }

    #[test]
    fn cleanup_removes_all_managed_rules_and_deletes_only_empty_results() {
        let input = format!("\u{feff}# User rules\r\n{IMAGE_AGENTS_BEGIN}\r\nold\r\n{IMAGE_AGENTS_END}\r\nKeep this.\r\n{IMAGE_AGENTS_BEGIN}\r\nold\r\n{IMAGE_AGENTS_END}\r\n");
        let after = remove_image_agents_rules(input.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&after).unwrap(),
            "\u{feff}# User rules\r\nKeep this.\r\n"
        );
        assert_eq!(remove_image_agents_rules(&after).unwrap().unwrap(), after);
        for input in [
            format!("\u{feff} \r\n{IMAGE_AGENTS_BEGIN}\r\nold\r\n{IMAGE_AGENTS_END}\r\n\t"),
            String::new(),
            "\u{feff}\r\n ".into(),
        ] {
            assert!(remove_image_agents_rules(input.as_bytes())
                .unwrap()
                .is_none());
        }
        assert!(remove_image_agents_rules(IMAGE_AGENTS_BEGIN.as_bytes()).is_err());
        assert!(remove_image_agents_rules(&[0xff]).is_err());
    }

    #[test]
    fn rules_preserve_bom_crlf_and_deduplicate() {
        let input = format!("\u{feff}# User rules\r\n\r\n{IMAGE_AGENTS_BEGIN}\r\nold\r\n{IMAGE_AGENTS_END}\r\n\r\nKeep this.\r\n{IMAGE_AGENTS_BEGIN}\r\nold again\r\n{IMAGE_AGENTS_END}\r\n");
        let output = ensure_image_agents_rules(input.as_bytes()).unwrap();
        assert!(output.starts_with(&[0xef, 0xbb, 0xbf]));
        let text = std::str::from_utf8(&output[3..]).unwrap();
        assert!(text.starts_with("# User rules\r\n\r\n"));
        assert!(text.contains("Keep this.\r\n"));
        assert!(!text.contains("old"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        assert_eq!(text.matches(IMAGE_AGENTS_BEGIN).count(), 1);
        assert_eq!(ensure_image_agents_rules(&output).unwrap(), output);
    }

    #[test]
    fn invalid_rules_do_not_change_config() {
        let temp = tempfile::tempdir().unwrap();
        let config = b"# keep\nmodel_provider = \"openai\"\n";
        fs::write(temp.path().join("config.toml"), config).unwrap();
        for input in [
            vec![0xff, 0xfe],
            format!("{IMAGE_AGENTS_BEGIN}\nunfinished").into_bytes(),
            format!("{IMAGE_AGENTS_END}\n").into_bytes(),
            format!("prefix {IMAGE_AGENTS_BEGIN}\n{IMAGE_AGENTS_END}\n").into_bytes(),
        ] {
            fs::write(temp.path().join("AGENTS.md"), &input).unwrap();
            assert!(configure(temp.path(), &temp.path().join("mcp.exe")).is_err());
            assert_eq!(fs::read(temp.path().join("config.toml")).unwrap(), config);
            assert_eq!(fs::read(temp.path().join("AGENTS.md")).unwrap(), input);
        }
    }

    #[test]
    fn second_file_failure_rolls_back_both_files() {
        for agents_existed in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let config = temp.path().join("config.toml");
            let agents = temp.path().join("AGENTS.md");
            fs::write(&config, b"original config").unwrap();
            if agents_existed {
                fs::write(&agents, b"original rules").unwrap();
            }
            let result = commit_codex_configuration_files_with(
                &config,
                b"new config",
                &agents,
                Some(b"new rules"),
                |path, contents| {
                    apply_file_update(path, contents)?;
                    if path == agents {
                        Err("injected second write failure".into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(fs::read(&config).unwrap(), b"original config");
            if agents_existed {
                assert_eq!(fs::read(&agents).unwrap(), b"original rules");
            } else {
                assert!(!agents.exists());
            }
        }
    }
}
