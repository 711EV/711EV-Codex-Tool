# GitHub 发布构建

推送与仓库版本一致的 `v<version>` 标签后运行 `workflows/release.yml`。CI 使用 Node.js 24，不执行会递增版本的本地 `npm run build`。

1. 发布预检确认 npm、Tauri、Cargo 和 Cargo.lock 版本一致、标签匹配、MCP 资源清单正确，并运行前端测试和类型检查。
2. 创建草稿 Release，然后 Windows 与 macOS 分别准备 Go（版本来自 `mcp/go.mod`）和 Rust，运行 Go 测试、构建本系统两种 MCP 架构，再执行 Rust 格式检查、测试和 Tauri 打包。Tauri 前置步骤也会重新生成 MCP，避免旧资源混入。
3. Windows 解包 NSIS，核对便携 EXE 与安装包内程序、双架构内嵌 MCP，并验证无 macOS 或重复资源；macOS 检查 App、DMG 和更新归档中的两种 Mach-O 架构、资源一致性及执行权限。两端都在临时目录用本地模拟服务验证 MCP 握手、生图、改图和凭据校验，不调用收费接口。
4. 两端验证通过后上传现有命名的签名附件，生成 `latest.json` 并发布。任一测试、构建或产物校验失败都不会执行最终发布。

MCP 随客户端升级，无需独立 Release 或额外下载工作流。提交时必须包含 `mcp/` 源码与依赖文件、构建及验证脚本、macOS 资源清单；生成的 `src-tauri/resources/mcp/` 二进制继续忽略，不提交 Git。

本地 Windows 构建同样执行内嵌资源和协议验证。设置 `SEVEN_ZIP_BINARY` 指向 7-Zip 可同时启用 NSIS 解包校验；CI 默认执行此项校验。

Windows 使用 `TAURI_SIGNING_PRIVATE_KEY` 生成安装包更新签名；macOS 使用同一密钥生成更新归档签名。保持现有附件命名、应用 identifier、可执行文件名和更新地址。
