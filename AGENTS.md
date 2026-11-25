# BDM 仓库代理指引

## 编译与交付

- 代码改动全部走 GitHub Actions 云端编译与检查，禁止在本机安装或保留 Rust 编译工具链（`rustup`、`cargo`、`rustc` 等）及本地编译产物目录。使用 `.github/workflows/ci.yml` 的 `Rust CI`，将改动提交到获准推送的分支。推送至 `main` 或创建、更新 PR 会触发工作流。
- 用 `gh run list --workflow ci.yml --commit <SHA>` 查找本次提交对应的运行，再用 `gh run watch <RUN_ID> --exit-status` 等待结束。失败时用 `gh run view <RUN_ID> --log-failed` 检查并修复相关问题，验证修复提交对应的运行。
- `build-windows` 使用 `cargo build --release --locked -p bdm-desktop -p bdm-agent` 编译 `bdm-desktop.exe` 与 `bdm.exe`。产物名称为 `bdm-desktop-windows`。完成后提供提交 SHA、运行链接以及产物链接，并报告 Windows 构建和其余 CI 检查各自的结果。
- 反重力的无界面操作使用 `bdm.exe mcp-serve`。魔搭中转使用 `scripts/modelscope_mcp.py`。配置见 [反重力 MCP 文档](docs/integrations/ANTIGRAVITY_MCP.md)。
- Token、本机 `config/modelscope.json`、`.env`、生产图库和 SQLite 数据留在本机。CI 验证使用测试数据。真实魔搭账号和图库传输需另行验证。
