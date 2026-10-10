# 配置与网络参考

使用 `xcosctl bootstrap` 生成的当前配置，保存在 `/etc/isarmg/xcos.env`，权限为 `0600`。它会填入实际安装版本路径；源码环境模板仅供字段名称参考。主要字段：

| 类别 | 变量 | 要求 |
|---|---|---|
| 数据 | `DATABASE_URL`、`RECORDINGS_DIR`、`XCOS_RUNTIME_DIR` | 使用固定外部绝对路径 |
| 身份 | `APP_JWT_SECRET` | 至少 32 字符随机值，主机秘密管理 |
| 凭据 | `CREDENTIALS_KEY` | Base64 编码的 32 字节随机值，持久保存在独立秘密系统 |
| 首管 | `BOOTSTRAP_ADMIN_USERNAME/PASSWORD` | 仅全新数据库初始化；默认 username 为 `admin` |
| 环境 | `APP_ENV=production` | 开发模式只允许 loopback |
| 登录 | xcss `AdministratorPolicyV1` 的 body/rate/Argon2 concurrency/timeout | 使用固定公共策略，不提供产品环境变量覆盖 |
| MediaMTX | API、playback、config、contract、binary | 必须指向同一固定 release |
| Web | 内嵌资源与 `share/web-assets.json` | 由实际 binary 精确验证，无生产目录覆盖 |
| 监听 | `BIND_ADDR=127.0.0.1:8080` | 代码默认值和正式样例一致；仅可信本机网关访问 |
| 公网播放 | `PUBLIC_HLS_BASE_URL`、`PUBLIC_WEBRTC_BASE_URL` | 保持同源相对路径，由 Caddy 路由 |
| Client 发布 | `PUBLIC_RTSP_PUBLISH_BASE_URL`、`MEDIAMTX_RTSP_CERT/KEY` | 必填 Client 可达的 RTSPS origin 与受信证书/私钥；生产拒绝明文和 loopback |
| WebRTC 地址 | `MEDIA_PUBLIC_HOSTS` | 填浏览器可达的服务器 DNS/IP，多个值用逗号分隔 |

MediaMTX 的 API `9997`、metrics `9998`、playback `9996` 固定 loopback。生产媒体监听 RTSPS `8322`、HLS
`8888`、WebRTC HTTP `8889` 和 WebRTC UDP `8189` 则绑定主机网卡：8322 是远程 xcoc 上传
视频的入口；8888/8889 通常只由本机 Caddy 代理；8189 是浏览器 WebRTC 媒体候选所需 UDP。防火墙应按
Client 网段、浏览器网络与 NAT 拓扑分别放行，这些媒体 listener 实际绑定主机网卡。摄像头本身仍应放入 Client 所在的隔离 VLAN，Server 不需要直达摄像头管理地址。
唯一代理源模板是 `deploy/Caddyfile`。它把 WHEP、HLS 和 Xcos 分别转发到本机
`127.0.0.1:8889`、`127.0.0.1:8888`、`127.0.0.1:8080`；9996/9997/9998 保持管理用途。
模板默认站点占位为 `:80`，不是生产 TLS 证明；生产必须在 Caddy 服务环境设置真实 `SITE_ADDRESS`（或由
配置管理渲染为真实站点），取得有效证书，并用防火墙阻止浏览器直连 Axum/MediaMTX。应用本身不终止
TLS，也不拒绝所有明文直连。若 Client 跨 NAT，必须为 8322 建立受限端口映射，并让
`PUBLIC_RTSP_PUBLISH_BASE_URL` 指向映射后的可达地址；WebRTC 的 8189/UDP 映射和 `MEDIA_PUBLIC_HOSTS`
也必须与浏览器实际访问路径一致。

安装或更新后至少执行：

```bash
# 由配置管理把 deploy/Caddyfile 的唯一站点块纳入主机受管配置；不要盲目覆盖其他站点。
sudoedit /etc/caddy/Caddyfile
sudo env SITE_ADDRESS=xcos.example.org \
  caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sudo systemctl reload caddy
```

主机使用拆分 include 目录时，将站点块纳入现有配置管理。
此配置按同机原生进程部署，反向代理上游使用明确的回环地址。

配置文件与私有状态由实际运行用户拥有，父路径仅允许 root 或运行用户写入。生产 RTSPS 使用 Client 可达的地址及受信任证书；密钥分别生成并通过主机秘密管理保存。

JSON/systemd 方案见[日常运维](operations.md#systemd-运行方式)，字段与环境映射的实现位于 `src/config.rs`。
