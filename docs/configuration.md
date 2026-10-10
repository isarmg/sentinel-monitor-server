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
| 监听 | `BIND_ADDR=127.0.0.1:8080` | 默认本机；生产可显式填写本机 LAN 地址，供远端网关回源；本机 `127.0.0.1:8080` 始终保留 |
| 公网播放 | `PUBLIC_HLS_BASE_URL`、`PUBLIC_WEBRTC_BASE_URL` | 保持同源相对路径，由 HTTPS 网关路由 |
| Client 发布 | `PUBLIC_RTSP_PUBLISH_BASE_URL`、`MEDIAMTX_RTSP_CERT/KEY` | 必填 Client 可达的 RTSPS origin 与受信证书/私钥；生产拒绝明文和 loopback |
| WebRTC 地址 | `MEDIA_PUBLIC_HOSTS` | 填浏览器可达的服务器 DNS/IP，多个值用逗号分隔 |

## 单机项目与远端网关

每个项目部署在一台机器上；不同项目可以分别部署在多台服务器，由路由系统上的统一网关按独立三级域名
转发。对 Xcos 而言，Rust 服务、MediaMTX、SQLite 和录像目录始终在同一台机器，网关可在本机或另一台机器。
浏览器通过该项目的完整 origin（例如 `https://xcos.example.org`）访问 Web、API、WHEP 和 HLS，不使用跨项目路径前缀。

`BIND_ADDR`（JSON 的 `bind_addr`）配置网关回源入口。默认 `127.0.0.1:8080` 不变；生产可显式设为
例如 `192.168.1.20:9080`，也可按部署需要使用通配地址。应用同时保留 `127.0.0.1:8080`，供固定的
MediaMTX 鉴权回调与本机就绪检查使用。修改回源端口不会释放本机 8080；该端口仍须可用。若配置
`0.0.0.0:8080`，一个监听已覆盖本机回调，不重复绑定。IPv6 监听独立，仍保留 IPv4 本机回调。
开发模式始终只允许 loopback。不要将回调、MediaMTX API 或数据库拆到其他主机。

公网 HTTPS 和证书由路由器或网关保证。应用通过普通 HTTP 接收回源，不探测代理品牌，也不依赖
`X-Forwarded-*` 来决定认证行为。网关须保留浏览器的公网 `Host` 与 `Origin`，不得把 `Host` 改为
私网 upstream 地址；生产仍使用 HTTPS 同源校验与 Secure Cookie。回源入口应仅对可信网关网络开放。

## 中央 Caddy 示例

`deploy/Caddyfile` 是可选网关示例，三个 upstream 的默认值仍分别为 `127.0.0.1:8080`、
`127.0.0.1:8889`、`127.0.0.1:8888`。Caddy 在另一台机器时，设置其服务环境，例如：

```sh
SITE_ADDRESS=xcos.example.org
XCOS_HTTP_UPSTREAM=192.168.1.20:9080
XCOS_WEBRTC_UPSTREAM=192.168.1.20:8889
XCOS_HLS_UPSTREAM=192.168.1.20:8888
```

这三个 upstream 指向同一台 Xcos 主机，HTTP 端口与 `BIND_ADDR` 一致。变量放在 Caddy 服务环境中，
站点块纳入路由器现有配置；也可直接在站点块中填写域名和地址。变量语法和 HTTP 回源保留 Host 的行为见
[Caddy 环境变量文档](https://caddyserver.com/docs/caddyfile/concepts#environment-variables)与
[反向代理请求头文档](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#headers)。

模板默认 `:80` 仅供起步；生产填写真实域名，由 Caddy 启用 HTTPS，或由上层路由器保证 HTTPS。
配置好服务环境后，在网关主机校验并重载：

```bash
# 在网关主机操作。
sudoedit /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sudo systemctl reload caddy
```

## 媒体网络

MediaMTX 的 API `9997`、metrics `9998`、playback `9996` 固定 loopback。生产媒体监听 RTSPS `8322`、
HLS `8888`、WebRTC HTTP `8889` 和 WebRTC UDP `8189` 则绑定主机网卡。8888/8889 供网关代理，
不得把 9996/9997/9998 加入公网代理。

RTSPS `8322` 是远程 xcoc 上传视频的独立入口，`8189/UDP` 是浏览器 WebRTC 媒体通道；二者不会经由
这个 HTTP Caddyfile 自动转发。按 Client 网段、浏览器网络与 NAT 拓扑配置对应路由/端口映射；
`PUBLIC_RTSP_PUBLISH_BASE_URL`、受信 RTSPS 证书和 `MEDIA_PUBLIC_HOSTS` 必须与实际可达地址一致。
HTTP 网关的 HTTPS 不替代 RTSPS 的 TLS。摄像头仍放在 Client 所在的隔离 VLAN，Server 不需要
直达摄像头管理地址。防火墙、NAT 和证书部署由运维在目标机器确认。

配置文件与私有状态由实际运行用户拥有，父路径仅允许 root 或运行用户写入。生产 RTSPS 使用 Client 可达的地址及受信任证书；密钥分别生成并通过主机秘密管理保存。

JSON/systemd 方案见[日常运维](operations.md#systemd-运行方式)，字段与环境映射的实现位于 `src/config.rs`。
