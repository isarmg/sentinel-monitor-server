# 06. MediaMTX、录像与播放链路

## 6.1 Companion 合同

Xcos 绑定精确 MediaMTX binary version/SHA 和规范 config。启动脚本验证普通文件、权限、内容和路径，
并通过 companion lock 保证单实例。系统包里的同名 binary 不自动可信。

## 6.2 锁顺序

应用通常持有数据库 instance、maintenance shared 和 runtime app lock；MediaMTX 使用 companion lock。
离线维护按 database maintenance -> runtime -> companion 取得排他锁。改变顺序会造成死锁或混合状态代。

## 6.3 摄像头到媒体服务器

摄像头 RTSP/ONVIF 凭据只由 Client 保管，Client 拉取摄像头后用短期媒体授权向 MediaMTX 发布；
Server 不直连摄像头，也不解密其设备密码。日志、进程参数、Web JSON 和审计都不能出现原始 RTSP 密码。
将摄像头放在 Client 所在的受限 VLAN，分别限制设备访问、Client RTSPS 发布和 MediaMTX 入站。

## 6.4 播放

浏览器先向 Xcos 获取短期授权，再经 TLS 代理访问 WHEP/HLS。代理路由必须保持 Host、真实 peer 和
WebSocket 握手语义，且不能公开 companion 管理 API。`isStreamTicket` 检查字段类型；实际 WHEP 请求在携带 Bearer 前，
通过 `requireSameOriginMediaUrl` 拒绝跨源和含用户名/密码的 URL，返回的 `Location` 也经 `whepResourceUrl` 同源校验。
同源绝对地址可以使用，生产模板仍采用同源相对路径以匹配 Caddy 路由。

当前原生部署模板只有 `deploy/Caddyfile`：`/media-webrtc/*`、`/media-hls/*` 和其余应用流量分别转发到
`127.0.0.1:8889`、`127.0.0.1:8888`、`127.0.0.1:8080`。这些是同主机进程端口，不是容器服务名；仓库
没有 Docker/Compose 运行模式，也没有 `app` 或 `mediamtx` DNS 兼容分支。

## 6.5 录像树

录像字节由 MediaMTX 写入固定 recordings directory。文件名、目录、mode、链接和容量都是备份/安全
合同的一部分。不能让 Web 输入直接变成未验证的物理路径。

## 6.6 容量管理

生产监控磁盘字节、inode、增长速率、录像时长和清理失败。预留空间必须覆盖 SQLite/WAL、录像写入、
必要的临时写入；磁盘满可能同时影响控制面和媒体面。

## 6.8 无画面排查顺序

1. 摄像头 RTSP 是否从 Client 主机可达，以及 Client 的 RTSPS 发布是否能到达 Server。
2. MediaMTX path/publisher 是否存在。
3. Xcos operation 是否成功而非 unknown。
4. 系统时间和播放授权是否有效。
5. 代理 WHEP/HLS/WebSocket/UDP 策略是否正确。
6. 浏览器 codec/网络错误。

## 6.9 变更规则

MediaMTX binary、SHA、config、start/doctor和release manifest必须保持同一已验证合同；运行不接受任意同名executable。
