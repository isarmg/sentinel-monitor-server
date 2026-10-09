# xcos 初学者学习指南

本手册按十章从摄像头、媒体面和控制面的基本概念，逐步进入持久操作、凭据、MediaMTX、测试与生产
运维。单页速览保留在章节索引之后，具体不变量和失败处置以专题章节为准。

1. [项目全景与版本边界](01-project-overview.md)
2. [开发环境与第一次运行](02-environment-and-first-run.md)
3. [Rust、视频链路与 Web 基础](03-rust-media-and-web-basics.md)
4. [服务端请求、认证与摄像头管理](04-server-request-and-camera-lifecycle.md)
5. [持久操作、协调器与故障恢复](05-operations-reconciler-and-recovery.md)
6. [MediaMTX、录像与播放链路](06-mediamtx-recording-and-playback.md)
7. [当前协议、加密与状态合同](07-current-contracts-and-cryptography.md)
8. [测试、调试与变更方法](08-testing-debugging-and-change-workflow.md)
9. [部署、安全与生产运维](09-deployment-security-and-operations.md)
10. [源码路线、练习与术语表](10-reading-roadmap-and-glossary.md)

以下内容是快速导读。

## 1. 先认识控制面和媒体面

摄像头视频不经过 Rust 应用转发。MediaMTX 连接 RTSP 摄像头并向浏览器提供 WHEP/HLS；Rust 应用保存
“应该有哪些 MediaMTX Path”的期望态，通过 MediaMTX 本地 API 协调实际态，并负责 Administrator 身份、
临时媒体授权和 ONVIF 控制。

```text
IP Camera --RTSP/ONVIF--> MediaMTX --WHEP/HLS--> Caddy --> Browser
                              ^                    |
                              | local API/auth     | /api/v1
                              +------ Rust/Axum <--+
                                        |
                                      SQLite
```

这种拆分让媒体协议交给专用组件，Rust 控制面保持可审计；代价是必须严格绑定 companion 版本、配置、
二进制摘要以及数据库与 MediaMTX 的最终一致性。

## 2. 目录阅读顺序

1. `web/src/protocol-contract.json`：浏览器、Rust 路由和 MediaMTX 回调共享的当前协议身份。
2. `src/main.rs`、`config.rs`、`routes.rs`：CLI、配置和 `/api/v1` 入口。
3. `auth.rs`、`login_security.rs`：用户 Session、CSRF、媒体 JWT 和登录保护。
4. `crypto.rs`：摄像机实例永久授权码的唯一当前 envelope。
5. `reconciliation.rs`、`mediamtx.rs`：期望态操作、租约和实际态协调。
6. `release.rs`、`deploy/*.sh`：固定发行树和原生生命周期；设备发现与厂商适配位于 xcoc。

## 3. 开发环境

需要 Rust `1.98`、Node/npm，以及可供集成测试使用的 Linux 工具。Web：

```bash
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config "$PWD/foundation-web-build.json" --mode development --no-install
```

开发启动必须显式设置 `APP_ENV=development` 和回环绑定；默认使用内嵌资源。需要目录热更新时设置
`XCSS_DEV_WEB_DIR="$PWD/web/dist"`，再运行：

```bash
read -rs -p 'Local administrator password: ' LOCAL_ADMIN_PASSWORD
printf '\n'
printf '%s\n' "$LOCAL_ADMIN_PASSWORD" | target/x86_64-unknown-linux-gnu/debug/xcos init --username admin
unset LOCAL_ADMIN_PASSWORD
target/x86_64-unknown-linux-gnu/debug/xcos config validate
target/x86_64-unknown-linux-gnu/debug/xcos run
```

正式 source-bound binary 拒绝未传 `--release-root` 的 `run`。不要在源码树保存生产 `.env`、MediaMTX binary 或凭据。

## 4. 浏览器登录

管理员初始身份只由显式`init --username`与stdin密码提供；原生`bootstrap --confirm-config`在配置审阅后调用同一入口。username candidate
是 1–64 bytes printable ASCII，经 trim ASCII/lowercase 后必须是 3–64 bytes、首尾字母数字、字符仅
`[a-z0-9._-]`；默认值为 `admin`，`@` 不合法。密码使用当前 Argon2id。登录成功后浏览器取得
`__Host-xcss-xcos-session` Secure/HttpOnly/SameSite=Strict Cookie；写请求同时需要 Session 绑定 CSRF。
登录按真实连接来源和规范化账户分别限流，并受请求体、Argon2 并发与超时预算保护。

控制面只有 Administrator：每个成功登录的账户都能访问摄像头、直播、录像、PTZ、事件、实例、审计
和系统状态。请求精确为 `{username,password}`，Session 精确包含
`authenticated/user_id/username/role/csrf_token` 五字段；`_xcss_administrators` 表不保存 email 或 `role`，wire 中固定的
`role:"admin"` 只是 Foundation 身份合同。实例授权码、Client 访问令牌和媒体 JWT
`actions` 都是数据面凭据或资源范围，不能解释为第二套控制面角色。

## 5. 实例授权码为什么是 envelope

每个摄像机实例的 36 位小写英文字母数字授权码以 Foundation secret envelope 的认证密文保存。专用
key 从 `CREDENTIALS_KEY` 派生；AAD 绑定授权实例 ID，因此密文不能复制到另一实例。设备
RTSP/ONVIF URL、用户名和密码只属于 Client，Server Schema 不存储这些字段。

当前实现从 `CREDENTIALS_KEY` 提供的唯一主密钥直接进行认证解密，不存储或比较独立的 key ID。
产品没有 previous key/keyring，也不接受合同外的 envelope 格式。`CREDENTIALS_KEY` 丢失意味着
已有授权码密文不可恢复。

## 6. 一次摄像机实例变更

管理员先创建一个授权实例；一个授权码只能配对一台摄像机。同一 Client 安装可保存多个授权码，
但会以独立实例分别配对、上报中立设备快照并向 MediaMTX 发布。Server 不提供摄像头
create/update/delete 或 ONVIF 发现 API。

授权码更换会立即失效旧 Client token，设备必须重新配对。删除已配对实例分两阶段：先撤销并
调和 MediaMTX 路径清理，确认成功后才原子删除摄像机状态和授权实例。

## 7. 播放和录像

浏览器先向当前 API 申请短时媒体 JWT，再通过同源 Caddy 入口访问 WHEP/HLS。当前前端 guard 只证明
ticket URL 是字符串，播放器也会接受绝对 URL/Location 并携带 Bearer；因此生产配置必须把
`PUBLIC_WEBRTC_BASE_URL` 保持为受审同源相对路径，不能把该约束误写成代码已强制。MediaMTX 调用唯一
`/internal/v1/media/auth` 校验。JWT 使用从 `APP_JWT_SECRET` 派生的当前签名 key，严格绑定 protocol、
issuer、audience、kind、camera、jti 与时间窗；不验证旧 token。

录像目录由 MediaMTX 写入；控制面通过 MediaMTX playback API 查询和代理授权播放，并没有本地录像
inventory 表或逐文件 Hash 索引。Web 不应直连 9996/9997/9998 管理/媒体端口。

## 8. 当前 Schema

数据库只由显式init在空目标中创建。普通run要求主文件存在，并以Foundation当前main/WAL/journal代的只读临时副本验证唯一
`product_metadata`、实际 `sqlite_schema` 指纹和 reconciler singleton 状态，再打开生产写连接。已有空
文件、非当前身份、额外列、非法租约或 Schema drift 都只读拒绝，不自动补表/补行。

Schema fingerprint framing、`product_metadata` DDL/列形状和exact-current identity比较来自Foundation `xcss-schema-identity 1.0.0`；稳定副本由公共SQLite模块捕获，Xcos只保留实际业务Schema、授权码和媒体租约不变量。这样多个产品不会复制同一 fingerprint 算法，也不会引入旧 Schema reader。

## 9. 修改代码的方法

- 路由变化：同步 Rust、Web contract、MediaMTX 配置和测试。
- 凭据合同：仅接受唯一当前envelope，生产者与消费者采用相同身份。
- 协调器变化：证明租约 owner、过期、续期、finalize 和 shutdown 的 fencing。
- MediaMTX合同：版本、SHA-256、lock、配置和lifecycle测试必须一致。
- 发行变化：更新全树 manifest 和重定位/篡改负例。

## 10. 术语

- **RTSP**：摄像头常用实时流输入协议。
- **ONVIF**：摄像头发现、能力和 PTZ 控制标准。
- **WHEP**：基于 HTTP 协商 WebRTC 播放的协议。
- **HLS**：基于分段 HTTP 的媒体播放协议。
- **PTZ**：Pan/Tilt/Zoom，云台水平、俯仰和变焦。
- **reconciliation**：把声明的期望态持续收敛为外部系统实际态。
- **fencing**：阻止过期 worker 在失去租约后提交结果的约束。
