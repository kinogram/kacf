use std::fs;
use std::path::Path;
use std::process::Command;

const BLUEPRINT_FILENAME: &str = "KACF_STARTER_BLUEPRINT.md";
const STARTER_DIRNAME: &str = "starter_source";

#[derive(Clone)]
struct StarterRepo {
    name: &'static str,
    url: &'static str,
    quickstart: &'static [&'static str],
    notes: &'static str,
}

#[derive(Clone)]
struct StarterBlueprint {
    scene: &'static str,
    reason: &'static str,
    repos: &'static [StarterRepo],
}

const STARTERS_ECOMMERCE: &[StarterRepo] = &[
    StarterRepo {
        name: "Medusa Starter",
        url: "https://github.com/medusajs/medusa",
        quickstart: &["npm install", "npm run dev"],
        notes: "电商后端能力成熟，适合快速搭建下单/支付/库存流程。",
    },
    StarterRepo {
        name: "Vendure",
        url: "https://github.com/vendure-ecommerce/vendure",
        quickstart: &["npm install", "npm run dev"],
        notes: "TypeScript 电商框架，插件生态完整。",
    },
];

const STARTERS_CMS: &[StarterRepo] = &[
    StarterRepo {
        name: "Strapi",
        url: "https://github.com/strapi/strapi",
        quickstart: &["yarn install", "yarn develop"],
        notes: "Headless CMS 生态成熟，适合内容管理后台。",
    },
    StarterRepo {
        name: "Ghost",
        url: "https://github.com/TryGhost/Ghost",
        quickstart: &["yarn install", "yarn dev"],
        notes: "博客/内容发布系统，开箱能力强。",
    },
];

const STARTERS_WORKFLOW: &[StarterRepo] = &[
    StarterRepo {
        name: "n8n",
        url: "https://github.com/n8n-io/n8n",
        quickstart: &["pnpm install", "pnpm start"],
        notes: "自动化编排成熟方案，适合业务流程自动化。",
    },
    StarterRepo {
        name: "Appsmith",
        url: "https://github.com/appsmithorg/appsmith",
        quickstart: &["docker compose up -d"],
        notes: "低代码平台，适合快速搭建内部工具。",
    },
];

const STARTERS_COMMUNITY: &[StarterRepo] = &[
    StarterRepo {
        name: "Chatwoot",
        url: "https://github.com/chatwoot/chatwoot",
        quickstart: &["docker compose up -d"],
        notes: "客服/工单/实时沟通一体化方案。",
    },
    StarterRepo {
        name: "Rocket.Chat",
        url: "https://github.com/RocketChat/Rocket.Chat",
        quickstart: &["docker compose up -d"],
        notes: "企业沟通平台，适合聊天协作产品。",
    },
];

const STARTERS_DEFAULT: &[StarterRepo] = &[
    StarterRepo {
        name: "Supabase",
        url: "https://github.com/supabase/supabase",
        quickstart: &["docker compose up -d"],
        notes: "后端即服务，账号/数据库/存储能力完整。",
    },
    StarterRepo {
        name: "Next.js",
        url: "https://github.com/vercel/next.js",
        quickstart: &["pnpm install", "pnpm dev"],
        notes: "前端与全栈基础设施成熟，生态广。",
    },
];

fn select_blueprint(goal: &str) -> StarterBlueprint {
    let g = goal.to_lowercase();
    if has_any(
        &g,
        &["电商", "商城", "shop", "store", "order", "payment", "sku"],
    ) {
        return StarterBlueprint {
            scene: "电商类产品",
            reason: "目标包含交易、商品、订单相关能力，优先复用成熟电商骨架。",
            repos: STARTERS_ECOMMERCE,
        };
    }
    if has_any(
        &g,
        &["博客", "内容", "cms", "新闻", "文章", "knowledge base"],
    ) {
        return StarterBlueprint {
            scene: "内容/CMS 类产品",
            reason: "目标包含内容管理或发布能力，优先复用成熟 CMS 平台。",
            repos: STARTERS_CMS,
        };
    }
    if has_any(
        &g,
        &[
            "自动化",
            "工作流",
            "workflow",
            "审批",
            "表单",
            "internal tool",
        ],
    ) {
        return StarterBlueprint {
            scene: "自动化/内部工具",
            reason: "目标强调流程自动化或内部系统，优先复用低代码与编排平台。",
            repos: STARTERS_WORKFLOW,
        };
    }
    if has_any(&g, &["聊天", "客服", "im", "chat", "社区", "forum", "消息"]) {
        return StarterBlueprint {
            scene: "沟通/社区类产品",
            reason: "目标强调实时消息与社区互动，优先复用成熟沟通系统。",
            repos: STARTERS_COMMUNITY,
        };
    }
    StarterBlueprint {
        scene: "通用 SaaS/Web 应用",
        reason: "目标不属于特定垂类，优先复用通用全栈底座。",
        repos: STARTERS_DEFAULT,
    }
}

fn has_any(text: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| text.contains(k))
}

fn workspace_has_user_code(workspace: &Path) -> bool {
    let Ok(read_dir) = fs::read_dir(workspace) else {
        return false;
    };
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "." || name == ".." {
            continue;
        }
        if name.starts_with(".autocoding_") {
            continue;
        }
        if name == BLUEPRINT_FILENAME || name == STARTER_DIRNAME {
            continue;
        }
        return true;
    }
    false
}

fn write_blueprint(workspace: &Path, goal: &str, picked: &StarterBlueprint, clone_note: &str) {
    let mut content = String::new();
    content.push_str("# KACF Smart Starter Blueprint\n\n");
    content.push_str("## 用户一句话目标\n\n");
    content.push_str(goal.trim());
    content.push_str("\n\n## 识别场景\n\n");
    content.push_str(picked.scene);
    content.push_str("\n\n## 选择理由\n\n");
    content.push_str(picked.reason);
    content.push_str("\n\n## 优先复用开源仓库\n\n");
    for (idx, repo) in picked.repos.iter().enumerate() {
        content.push_str(&format!("{}. **{}**\n", idx + 1, repo.name));
        content.push_str(&format!("   - Repo: `{}`\n", repo.url));
        content.push_str(&format!(
            "   - Quickstart: `{}`\n",
            repo.quickstart.join(" && ")
        ));
        content.push_str(&format!("   - Note: {}\n", repo.notes));
    }
    content.push_str("\n## 自动开局状态\n\n");
    content.push_str(clone_note);
    content.push_str(
        "\n\n## 执行要求（给 AI）\n\n- 优先在现有成熟项目基础上改造，而不是从零重写。\n- 面向完全不懂电脑的小白：默认配置可直接启动。\n- 每一轮都必须产出可验证结果（启动、测试、可访问页面）。\n",
    );
    let _ = fs::write(workspace.join(BLUEPRINT_FILENAME), content);
}

fn try_clone_primary(workspace: &Path, repo_url: &str) -> String {
    let target = workspace.join(STARTER_DIRNAME);
    if target.exists() {
        return format!(
            "已存在 `{}`，跳过自动克隆（保留现有内容）。",
            target.display()
        );
    }
    let output = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(repo_url)
        .arg(&target)
        .output();
    match output {
        Ok(out) if out.status.success() => {
            format!("已自动克隆主推荐仓库到 `{}`。", target.display())
        }
        Ok(out) => format!(
            "自动克隆失败（exit={}），可后续手动执行 git clone。stderr: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("自动克隆未执行成功：{}", e),
    }
}

pub(crate) fn bootstrap_workspace_for_goal(workspace: &Path, goal: &str) -> Option<String> {
    let trimmed_goal = goal.trim();
    if trimmed_goal.is_empty() {
        return None;
    }
    if fs::create_dir_all(workspace).is_err() {
        return None;
    }
    let picked = select_blueprint(trimmed_goal);
    let clone_note = if workspace_has_user_code(workspace) {
        "检测到工作区已有用户代码，未执行自动克隆，仅生成开局蓝图。".to_string()
    } else {
        try_clone_primary(workspace, picked.repos[0].url)
    };
    write_blueprint(workspace, trimmed_goal, &picked, &clone_note);
    let mut inject = String::new();
    inject.push_str("\n[SMART_STARTER]\n");
    inject.push_str("你必须优先复用成熟开源项目，不要从零造轮子。\n");
    inject.push_str(&format!("识别场景：{}。\n", picked.scene));
    inject.push_str(&format!("选择理由：{}。\n", picked.reason));
    inject.push_str("候选仓库（按优先级）：\n");
    for repo in picked.repos {
        inject.push_str(&format!("- {} ({})\n", repo.name, repo.url));
    }
    inject.push_str(&format!(
        "工作区开局蓝图文件：`{}`。\n",
        workspace.join(BLUEPRINT_FILENAME).display()
    ));
    inject.push_str("目标用户是完全不懂电脑的小白：默认配置应可直接运行。\n");
    Some(inject)
}

#[cfg(test)]
mod tests {
    use super::{bootstrap_workspace_for_goal, select_blueprint};

    #[test]
    fn blueprint_selects_ecommerce() {
        let b = select_blueprint("帮我做一个电商商城，带订单和支付");
        assert_eq!(b.scene, "电商类产品");
        assert!(b.repos[0].url.contains("medusa"));
    }

    #[test]
    fn bootstrap_returns_injected_goal_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("starter_source")).expect("create starter_source");
        let inject = bootstrap_workspace_for_goal(dir.path(), "做一个给小白用的任务管理 SaaS");
        assert!(inject.is_some());
        let text = inject.unwrap_or_default();
        assert!(text.contains("[SMART_STARTER]"));
    }
}
