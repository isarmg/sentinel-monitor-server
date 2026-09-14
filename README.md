# 哨界 Sentinel Monitor

Sentinel Monitor `0.2.10` 是只支持 `x86_64-unknown-linux-gnu` 物理机原生部署的浏览器摄像头监控系统。Rust/Axum 控制面
负责用户、客户端实例、摄像头、PTZ、审计和期望态；固定版本的 MediaMTX companion 负责 RTSP 接入、WHEP/HLS
播放和服务端录像；SQLite 保存当前业务状态。独立 `sentinel-monitor-client` 在摄像头所在主机管理 RTSP 凭据、向 Server 发布视频，并可按摄像头选择录像保存在客户端或服务端。

Client 使用可扩展设备适配器统一不同品牌：当前 `rtsp` 适配器接入已知码流地址，`onvif` 适配器发现设备并读取
厂商、型号、媒体配置、主/子码流与 PTZ 能力。Server 只保存统一身份、能力、码流描述和健康状态，不保存 Client
摄像头的局域网地址或密码；Client 摄像头的 PTZ 通过有期限的统一命令下发并回报执行结果。

每个 Sentinel Client 实例拥有一个长期授权码。Server 同时保存用于匹配的摘要和可供管理员查看的认证加密密文；首次配对不消耗授权码。管理员更换授权码会立即撤销现有客户端访问凭据、停止其发布路径并将实例恢复为待配对，Client 必须使用新码显式重新配对。

产品只理解当前 `0.2.10` Schema、`/api/v2` 协议、凭据 envelope 和固定发行树，不读取其他代数据库、
密文、runtime 或配置，也不提供迁移、备份和恢复命令。稳定版本形成后的代际变更才会交给
`sarmg-upgrade`；当前开发阶段没有历史升级 edge。

浏览器源码统一位于 `clients/web/`；可提交的环境样例和受审 MediaMTX 配置统一位于 `config/`；主机侧
代理模板统一位于 `deploy/`。真实 credentials、运行环境文件和录像不进入源码仓库。生产环境文件唯一位置是
`/etc/isarmg/sentinel-monitor.env`。本仓库刻意不提供 systemd unit；正式生命周期由不可变发行树内的
`native/sentinelctl bootstrap|start|status|stop` 统一管理；内部动作模块不可直接执行。

Sentinel Rust 控制面以及 MediaMTX 的 API、metrics、playback 管理端口只监听 loopback。媒体监听不同：
生产启动器强制 RTSPS `:8322`，HLS `:8888`、WebRTC HTTP `:8889` 和 WebRTC UDP `:8189` 按 MediaMTX 合同监听主机网卡。
生产应由 `deploy/Caddyfile` 把浏览器 HLS/WebRTC HTTP 与控制面汇聚到同一 TLS origin，同时用防火墙仅向
配对 Client 开放 RTSPS 发布入口，并按网络拓扑开放 WebRTC UDP；不得公开 9996/9997/9998。配置中的
`PUBLIC_RTSP_PUBLISH_BASE_URL` 必须是 Client 实际可达、证书受信且名称匹配的 `rtsps://` origin，生产不能使用明文 RTSP 或 loopback。发布 URL 只携带 120 秒、单摄像头/单 profile/publish 限定的媒体 Token，不复用长期 Client API 凭据。

## 快速验证

```bash
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 check --locked --all-targets
cargo +1.98.0 clippy --locked --all-targets -- -D warnings
cargo +1.98.0 test --locked --all-features
cd clients/web && npm ci && npm run check:foundation && npm run build
./native/lifecycle-test.sh
./native/relocated-smoke-test.sh
```

Web 使用 Node `26.7.0`、React/React DOM `19.2.8`、TypeScript strict `5.8.3` 和 Vite `7.3.6`；
`build` 自身会先执行 `check:foundation`，单独列出该命令是为了让依赖漂移更早、更易定位。控制面只有
Administrator 身份，认证端点固定为 `/api/v2/auth/login`、`/api/v2/auth/session` 和
`/api/v2/auth/logout`，不实现路径别名或分级身份。

Server 管理身份采用 Foundation 当前 username 合同：登录精确为 `{username,password}`，Session 精确为
`{authenticated,user_id,username,role:"admin",csrf_token}`。内置 React/Vite Web 的登录、当前管理员和
管理员 CRUD 同步使用 username；`users` 表不再含 email。摄像头 RTSP/ONVIF username、加密密码、媒体
JWT/camera identity 与浏览器媒体逻辑仍是原有数据面合同，不能与 Administrator username 混用。

## 文档

- [文档总览](docs/README.md)
- [初学者学习指南](docs/beginner-guide/README.md)
- [项目工作流程与流程树](docs/project-workflow.md)
- [完整功能与取舍清单](docs/feature-inventory-and-tradeoffs.md)
- [原生部署、安全、诊断与故障运维](docs/operations.md)

账号修改方法见 [账号设置](docs/account-settings.md)。
