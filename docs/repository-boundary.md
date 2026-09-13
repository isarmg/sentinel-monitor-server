# Server 仓库

当前目录及 GitHub 仓库为 `sentinel-monitor-server`：https://github.com/isarmg/sentinel-monitor-server 。
服务端及管理 Web 同属本仓库；摄像头主机侧代理位于相邻的独立 `sentinel-monitor-client` 项目。Client 保存摄像头地址与凭据，通过 RTSP/ONVIF 及后续厂商适配器解析设备，发布 RTSP、执行客户端录像和设备控制；Server 只保存客户端上报的统一身份、能力、码流描述、健康状态、实例授权材料、短期命令和服务端录像。
历史提交和发行标签保留，产品可执行文件及运行数据路径不因仓库改名而改写。
