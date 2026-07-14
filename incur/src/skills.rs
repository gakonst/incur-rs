use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Error, InternalResult as Result, Manifest};

pub(crate) fn stale_cta(manifest: &Manifest) -> Option<crate::CtaBlock> {
    let path = metadata_path(&manifest.name)?;
    let metadata = serde_json::from_slice::<Value>(&fs::read(path).ok()?).ok()?;
    let installed = metadata
        .get("paths")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .any(|path| Path::new(path).join("SKILL.md").exists());
    if !installed || metadata.get("hash").and_then(Value::as_str) == Some(&manifest_hash(manifest))
    {
        return None;
    }
    Some(
        crate::CtaBlock::new([crate::Cta::new("skills add").description("sync outdated skills")])
            .description("Skills are out of date:"),
    )
}

pub(crate) fn install(manifest: &Manifest, global: bool, depth: usize) -> Result<Vec<String>> {
    let base = if global {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| Error::new("HOME_NOT_FOUND", "HOME is not set"))?;
        home.join(".agents/skills")
    } else {
        std::env::current_dir()?.join(".agents/skills")
    };
    let files = generate(manifest, depth);
    let mut paths = Vec::new();
    for (name, content) in files {
        let directory = base.join(sanitize(&name));
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("SKILL.md"), format!("{}\n", content.trim_end()))?;
        paths.push(directory.display().to_string());
        link_detected_agents(&directory, &name, global);
    }
    write_metadata(manifest, &paths)?;
    Ok(paths)
}

pub(crate) fn list(manifest: &Manifest, depth: usize) -> Value {
    Value::Array(
        generate(manifest, depth)
            .into_iter()
            .map(|(name, content)| {
                json!({
                    "name": name,
                    "description": frontmatter_description(&content),
                    "installed": installed(&name),
                })
            })
            .collect(),
    )
}

fn generate(manifest: &Manifest, depth: usize) -> Vec<(String, String)> {
    if depth == 0 {
        return vec![(manifest.name.clone(), render(manifest, &manifest.commands, &manifest.name))];
    }
    let mut groups = std::collections::BTreeMap::<String, Vec<_>>::new();
    for command in &manifest.commands {
        let segments = command.name.split_whitespace().take(depth).collect::<Vec<_>>();
        let key = if segments.is_empty() {
            manifest.name.clone()
        } else {
            format!("{}-{}", manifest.name, segments.join("-"))
        };
        groups.entry(key).or_default().push(command.clone());
    }
    groups
        .into_iter()
        .map(|(name, commands)| {
            let content = render(manifest, &commands, &name);
            (name, content)
        })
        .collect()
}

fn render(manifest: &Manifest, commands: &[crate::CommandInfo], skill_name: &str) -> String {
    let description = if commands.len() == 1 {
        commands[0]
            .description
            .clone()
            .or_else(|| manifest.description.clone())
            .unwrap_or_else(|| format!("Use the {} CLI", manifest.name))
    } else {
        manifest.description.clone().unwrap_or_else(|| format!("Use the {} CLI", manifest.name))
    };
    let frontmatter_description = format!(
        "{}. Run `{} --help` for usage details.",
        description.trim_end_matches('.'),
        manifest.name
    );
    let mut output = format!(
        "---\nname: {}\ndescription: {}\nrequires_bin: {}\ncommand: {}\n---\n",
        sanitize(skill_name),
        yaml_scalar(&frontmatter_description),
        yaml_scalar(&manifest.name),
        yaml_scalar(&manifest.name),
    );
    for command in commands {
        output.push_str(&format!("\n# {}\n", command.signature(&manifest.name)));
        if let Some(description) = &command.description {
            output.push_str(&format!("\n{description}\n"));
        }
        if command.annotations.as_ref().and_then(|annotations| annotations.destructive_hint)
            == Some(true)
        {
            output
                .push_str("\n> Confirm with the user before executing this destructive command.\n");
        }
        if let Some(instructions) = &command.instructions {
            output.push_str(&format!("\n> {instructions}\n"));
        }
        output.push_str("\n## Inputs\n\n```json\n");
        output.push_str(&serde_json::to_string_pretty(&command.input_schema).unwrap_or_default());
        output.push_str("\n```\n\n## Output\n\n```json\n");
        output.push_str(&serde_json::to_string_pretty(&command.output_schema).unwrap_or_default());
        output.push_str("\n```\n");
    }
    output
}

fn yaml_scalar(value: &str) -> String {
    // JSON strings are valid YAML scalars and avoid pulling the YAML formatter into `skills`.
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

fn sanitize(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(
            |character| {
                if character.is_ascii_alphanumeric() || character == '-' { character } else { '-' }
            },
        )
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

fn frontmatter_description(content: &str) -> Option<&str> {
    content.lines().find_map(|line| line.strip_prefix("description: "))
}

fn installed(name: &str) -> bool {
    let global = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".agents/skills").join(sanitize(name)).join("SKILL.md"));
    let project = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".agents/skills").join(sanitize(name)).join("SKILL.md"));
    global.is_some_and(|path| path.exists()) || project.is_some_and(|path| path.exists())
}

fn link_detected_agents(canonical: &Path, name: &str, global: bool) {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let cwd = std::env::current_dir().unwrap_or_default();
    let candidates = [
        (claude_home.clone(), claude_home.join("skills"), ".claude/skills"),
        (home.join(".codeium/windsurf"), home.join(".codeium/windsurf/skills"), ".windsurf/skills"),
        (home.join(".continue"), home.join(".continue/skills"), ".continue/skills"),
        (home.join(".roo"), home.join(".roo/skills"), ".roo/skills"),
        (home.join(".kilocode"), home.join(".kilocode/skills"), ".kilocode/skills"),
        (config_home.join("goose"), config_home.join("goose/skills"), ".goose/skills"),
        (home.join(".augment"), home.join(".augment/skills"), ".augment/skills"),
        (home.join(".trae"), home.join(".trae/skills"), ".trae/skills"),
        (home.join(".junie"), home.join(".junie/skills"), ".junie/skills"),
        (config_home.join("crush"), config_home.join("crush/skills"), ".crush/skills"),
        (home.join(".kiro"), home.join(".kiro/skills"), ".kiro/skills"),
        (home.join(".qwen"), home.join(".qwen/skills"), ".qwen/skills"),
        (home.join(".openhands"), home.join(".openhands/skills"), ".openhands/skills"),
    ];
    for (detector, global_path, project_path) in candidates {
        if !detector.exists() {
            continue;
        }
        let agent_base = if global { global_path } else { cwd.join(project_path) };
        let link = agent_base.join(sanitize(name));
        let _ = fs::create_dir_all(&agent_base);
        if !link_available(&link) {
            continue;
        }
        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(canonical, &link).is_err() {
                let _ = copy_dir(canonical, &link);
            }
        }
        #[cfg(windows)]
        {
            let _ = std::os::windows::fs::symlink_dir(canonical, &link)
                .or_else(|_| copy_dir(canonical, &link));
        }
    }
}

fn link_available(link: &Path) -> bool {
    match link.symlink_metadata() {
        // Never delete a user's real directory, file, or symlink. A colliding path belongs to the
        // user and wins, even when it is a stale link.
        Ok(_) => false,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn write_metadata(manifest: &Manifest, paths: &[String]) -> Result<()> {
    let path = metadata_path(&manifest.name)
        .ok_or_else(|| Error::new("HOME_NOT_FOUND", "HOME is not set"))?;
    let directory = path
        .parent()
        .ok_or_else(|| Error::new("INVALID_PATH", "skills metadata path has no parent"))?;
    fs::create_dir_all(directory)?;
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string(&json!({"hash": manifest_hash(manifest), "paths": paths}))?
        ),
    )?;
    Ok(())
}

fn metadata_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    Some(data.join("incur").join(format!("{}.json", sanitize(name))))
}

fn manifest_hash(manifest: &Manifest) -> String {
    let bytes = serde_json::to_vec(&manifest.commands).unwrap_or_default();
    let hash = format!("{:x}", Sha256::digest(bytes));
    hash[..16].to_owned()
}

fn copy_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{generate, link_available};

    #[test]
    fn never_replaces_a_user_owned_skill_directory() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("existing-skill");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("SKILL.md"), "user content").unwrap();

        assert!(!link_available(&directory));
        assert_eq!(std::fs::read_to_string(directory.join("SKILL.md")).unwrap(), "user content");
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn generated_skill_frontmatter_is_valid_yaml() {
        let manifest = crate::Manifest {
            name: "example-cli".to_owned(),
            version: None,
            description: Some("Manage values: safely".to_owned()),
            commands: Vec::new(),
        };
        let (_, content) = generate(&manifest, 0).pop().unwrap();
        let frontmatter = content.split("---").nth(1).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(frontmatter).unwrap();

        assert_eq!(value["requires_bin"], "example-cli");
        assert!(value["description"].as_str().unwrap().contains("Manage values: safely"));
    }
}
