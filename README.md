# xcos

xcos `1.0.0` 是自托管的浏览器摄像头监控系统。1.0.0 候选采用 SQLx 0.9、Rust 1.99，并按 CLI、服务组装、数据库和 HTTP 业务职责整理模块；完整依赖与正式发行验收仍在执行。Rust/Axum 控制面负责管理员、Client 实例、摄像头状态、PTZ 和录像索引；固定版本的 MediaMTX companion 负责视频接入、播放与 Server 侧录像。

正式 Server 仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`）。摄像头地址和密码保存在独立的 xcoc，Server 只管理统一设备状态和短期媒体发布授权。

设备侧从安装、配对/重新配对到服务或后台任务管理、诊断与卸载，见独立 [Client 分平台部署指南](https://github.com/isarmg/xcoc/blob/main/docs/platform-setup.md)。

## 配置概览

安装发行树后，由生命周期脚本创建生产环境文件，再编辑其中的密钥、管理员密码、公开媒体地址和证书路径：

```sh
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap
sudoedit /etc/isarmg/xcos.env
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap --confirm-config
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl status
```

`bootstrap`只生成私有配置；`bootstrap --confirm-config`审阅后显式执行`init`并只读校验既有状态，成功后移除临时初始密码。`start`仅运行当前已初始化数据，不能自动建库或创建管理员。核心CLI为`init`、`run`、`config validate`和`status`；`--config`读取私有JSON，显式CLI覆盖环境、文件和默认值，`--json`提供单条机器错误。

当前数据格式为`xcos-db-v1`（Schema1），Client设备协议为`xcos-edge-v1`，能力使用`supported/unsupported/unknown`三态；浏览器wire身份为`xcos-wire-v1`（HTTP路径前缀仍为`/api/v1`），媒体JWT为v1，几种身份不可混用。普通运行只接受当前格式，非当前输入明确拒绝且不改写。

控制面和 MediaMTX 管理端口应只监听 loopback。生产入口由 HTTPS 反向代理提供；RTSPS 发布地址及证书必须能被所有 Client 验证。网络端口、反向代理和录像目录配置见[运维文档](docs/operations.md)。

## 开发验证

```sh
cargo +1.99.0 fmt --all -- --check
cargo +1.99.0 clippy --locked --all-targets -- -D warnings
cargo +1.99.0 test --locked --all-features
(cd web && npm ci && npm run build)
./scripts/lifecycle-test.sh
```

## 文档

- [文档总览](docs/README.md)
- [初学者指南](docs/beginner-guide/README.md)
- [项目工作流程](docs/project-workflow.md)
- [功能范围与取舍](docs/feature-inventory-and-tradeoffs.md)
- [部署与运维](docs/operations.md)

许可与第三方组件信息以发行包内的许可证清单为准。

当前发布版本：**1.0.0**。参见 [1.0.0 发布说明](docs/releases/1.0.0.md)和[项目命名](docs/naming.md)。

公共支撑的职责、单体依赖、平台边界与验证方法见[公共支撑说明](docs/common-support.md)。
