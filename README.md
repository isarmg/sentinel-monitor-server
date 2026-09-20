# Sentinel Monitor Server

Sentinel Monitor `0.2.14` 是自托管的浏览器摄像头监控系统。Rust/Axum 控制面负责管理员、Client 实例、摄像头状态、PTZ 和录像索引；固定版本的 MediaMTX companion 负责视频接入、播放与 Server 侧录像。

正式 Server 仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`）。摄像头地址和密码保存在独立的 Sentinel Client，Server 只管理统一设备状态和短期媒体发布授权。

## 配置概览

安装发行树后，由生命周期脚本创建生产环境文件，再编辑其中的密钥、管理员密码、公开媒体地址和证书路径：

```sh
sudo /opt/isarmg/sentinel-monitor/releases/0.2.14/native/sentinelctl bootstrap
sudoedit /etc/isarmg/sentinel-monitor.env
sudo /opt/isarmg/sentinel-monitor/releases/0.2.14/native/sentinelctl bootstrap --confirm-config
sudo /opt/isarmg/sentinel-monitor/releases/0.2.14/native/sentinelctl start
sudo /opt/isarmg/sentinel-monitor/releases/0.2.14/native/sentinelctl status
```

控制面和 MediaMTX 管理端口应只监听 loopback。生产入口由 HTTPS 反向代理提供；RTSPS 发布地址及证书必须能被所有 Client 验证。网络端口、反向代理和录像目录配置见[运维文档](docs/operations.md)。

## 开发验证

```sh
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 clippy --locked --all-targets -- -D warnings
cargo +1.98.0 test --locked --all-features
(cd web && npm ci && npm run build)
./native/lifecycle-test.sh
```

## 文档

- [文档总览](docs/README.md)
- [初学者指南](docs/beginner-guide/README.md)
- [项目工作流程](docs/project-workflow.md)
- [功能范围与取舍](docs/feature-inventory-and-tradeoffs.md)
- [部署与运维](docs/operations.md)

许可与第三方组件信息以发行包内的许可证清单为准。
