# xcos 开发与验证

Rust/Axum 控制面按 CLI、服务组装、数据库和 HTTP 业务职责分工。当前使用 Rust 1.99.0、SQLx 0.9；
Web 使用 Node.js 26.7.0。构建和运行目标固定 Linux AMD64 GNU。当前源码和公开历史发行包分别按自身
数据库合同运行；不要把发布成功视为其后的代码已包含在同名发行资产中。

## 开发验证

```sh
cargo +1.99.0 fmt --all -- --check
cargo +1.99.0 clippy --locked --all-targets -- -D warnings
cargo +1.99.0 test --locked --all-features
(cd web && npm ci && npm run build)
./scripts/lifecycle-test.sh
```

发行包、MediaMTX lock、私有配置、初始化与端口要求见[运维文档](operations.md)。设备侧安装、
配对、服务管理、诊断与卸载见 [xcoc 平台指南](https://github.com/isarmg/xcoc/blob/main/docs/platform-setup.md)。

## 协议与初始化边界

核心 CLI 为 `init`、`run`、`config validate` 和 `status`；`--config` 读取私有 JSON，显式 CLI
覆盖环境、文件和默认值，`--json` 提供单条机器错误。普通运行只接受当前格式，非当前输入拒绝且不改写。

当前数据格式为 `xcos-db-v2`（Schema 2），Client 协议为 `xcos-edge-v1`，能力采用
`supported/unsupported/unknown` 三态；浏览器 wire 身份为 `xcos-wire-v2`（HTTP 前缀仍为
`/api/v1`），媒体 JWT 为 v1。这些身份独立管理，不可互换。

状态读取失败时的 `fresh/stale/unknown` 观测合同及安全要求见[运维文档](operations.md#摄像机状态与观测时效)。
许可与第三方组件信息以发行包内的许可证清单为准。
