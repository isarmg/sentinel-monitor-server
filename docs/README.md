# xcos 文档

先完成服务端部署，再配对摄像头客户端。本文档面向当前源码；公开 v1.0.0 归档与当前源码的数据库格式不同，部署时按实际程序和同一提交的文档操作。

## 开始使用

1. [安装与首次运行](getting-started.md)：准备主机、配置 HTTPS/RTSPS、初始化并检查服务。
2. [添加摄像头与日常使用](usage.md)：创建实例、配对 xcoc、播放、录像和查看日志。
3. [配置与运维](operations.md)：配置字段、systemd、健康检查与安全事件。
4. [故障排查](troubleshooting.md)：从症状定位客户端、控制面或媒体面。

## 开发与参考

- [开发与验证](development.md)：工具链、构建、测试和发行检查。
- [运行状态与协议参考](runtime-reference.md)：观测时效、操作结果、数据库与历史容量。
- [请求和媒体流程](project-workflow.md)、[功能设计参考](feature-inventory-and-tradeoffs.md)。
- [从零读懂 xcos](beginner-guide/README.md)：按章节学习 Rust、Web 和媒体服务。
- [账号设置](account-settings.md)、[仓库职责](repository-boundary.md)、[公共支撑](common-support.md)、[安全审查](unsafe-audit.md)。
- [1.0.0 发行记录](releases/1.0.0.md)、[项目首页](../README.md)。
