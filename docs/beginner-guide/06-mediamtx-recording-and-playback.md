# 06. MediaMTX、录像与播放链路

## 6.1 Companion 合同

Xcos 绑定精确 MediaMTX binary version/SHA 和规范 config。启动脚本验证普通文件、权限、内容和路径，
并通过 companion lock 保证单实例。系统包里的同名 binary 不自动可信。

## 6.2 锁顺序

应用通常持有数据库 instance、maintenance shared 和 runtime app lock；MediaMTX 使用 companion lock。
离线维护按 database maintenance -> runtime -> companion 取得排他锁。改变顺序会造成死锁或混合状态代。

## 6.3 摄像头到媒体服务器

credential 在 Server 内解密，只在调用/生成受保护配置的最短范围存在。日志、进程参数、Web JSON 和
审计都不能出现原始 RTSP 密码。网络上应把摄像头放在受限 VLAN，并限制 MediaMTX 出站/入站。

## 6.4 播放

浏览器先向 Xcos 获取短期授权，再经 TLS 代理访问 WHEP/HLS。代理路由必须保持 Host、真实 peer 和
WebSocket握手语义，且不能公开 companion 管理 API。当前 `isStreamTicket` 只校验 URL 字段类型，WHEP player
对绝对 ticket/Location 也会携带 Bearer；生产必须把 WebRTC base URL 锁定为同源相对路径，直到客户端
实现显式 same-origin 拒绝。

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

1. 摄像头 RTSP 是否从 MediaMTX 主机可达。
2. MediaMTX path/publisher 是否存在。
3. Xcos operation 是否成功而非 unknown。
4. 系统时间和播放授权是否有效。
5. 代理 WHEP/HLS/WebSocket/UDP 策略是否正确。
6. 浏览器 codec/网络错误。

## 6.9 变更规则

MediaMTX binary、SHA、config、start/doctor和release manifest必须保持同一已验证合同；运行不接受任意同名executable。
