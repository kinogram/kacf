# KACF (Kinogram AutoCoding Framework)

这是当前工作区对应代码的 README。
文档只描述当前代码已实现行为，不推测其他分支或历史版本。

## 项目定位

KACF 是一个 Web UI 驱动的自动编程框架：
输入目标后，系统执行「生成补丁 -> 应用补丁 -> 运行评测 -> 根据结果继续迭代」的闭环。

当前代码特征（以本工作区为准）：
- 单二进制启动（后端 + Web UI）
- 项目化工作流（新建/切换/删除）
- 自动保存与断点恢复
- 无人值守运行（自动恢复次数、停止时间）
- SSE 实时事件流 + 轮询回退
- Diff 独立页面展示（主页面不渲染大 diff）

## 快速开始

### 环境要求

- Rust stable
- Linux/macOS（Windows 需自行验证）
- 可用模型 API Key（默认是 DeepSeek 兼容配置）

### 启动

```bash
cargo run
```

默认监听：`0.0.0.0:8080`

指定端口：

```bash
AUTOCODING_PORT=18080 cargo run
```

### Release 运行

```bash
cargo build --release
./target/release/kacf
```

## 使用流程（当前 UI）

1. 输入目标需求（Goal）
2. 点击“运行当前项目”
3. 查看状态与日志
4. 需要时“从断点恢复”
5. 在全局配置中调整语言、自动恢复次数、停止时间、日志上限

说明：
- 项目配置以自动保存为主。
- Diff 使用“查看diff ↗”在新标签页打开。

## 主要接口

### 页面与静态资源

- `GET /`
- `GET /diff`
- `GET /assets/app.css`
- `GET /assets/app.js`
- `GET /assets/diff.js`
- `GET /assets/languages/list`
- `GET /assets/languages/{code}.json`

### 运行控制

- `POST /start`
- `POST /resume`
- `POST /stop`
- `POST /clarify`
- `POST /revert`
- `POST /push`

### 项目与状态

- `GET /projects`
- `POST /projects`
- `DELETE /projects/{id}`
- `POST /projects/suggest_slug`
- `GET /project_config`
- `GET /ui_state`
- `GET /ui_cache`
- `PUT /ui_cache`
- `GET /diff_data`

### 可观测性

- `GET /health`
- `GET /metrics`
- `GET /events`
- `GET /events/stream`
- `POST /debug/client_logs`
- `GET /debug/client_logs`

## 目录结构

```text
.
├── src/
├── static/
│   ├── index.html
│   ├── diff.html
│   ├── app.css
│   ├── js/
│   └── languages/
├── scripts/
│   ├── release_check.sh
│   ├── smoke_web.sh
│   └── metrics_gate.sh
├── autocoding_data/
└── LICENSE
```

## 开发检查

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

发布检查：

```bash
bash scripts/release_check.sh
```

## 语言包规则

启动时会严格校验 `static/languages`：
- 文件名格式：`KACF_<language_code>_<pack_version>.json`
- 必须包含 `__meta`
- `language_code`、`pack_version`、`inputer_version` 与程序要求一致
- 全部语言包 key 集合必须和 `en` 完全一致

任一项不满足，服务会拒绝启动。

## 许可证

使用仓库中的自定义许可证：`LICENSE`（KACF Personal & Non-Commercial License 1.1）。

商业授权联系：`gregsons334@gmail.com`
