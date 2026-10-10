# xcos 文档总览

本文档集只描述 `1.0.0` 当前实现。协议 JSON、当前 Schema、MediaMTX lock、发行 manifest 和自动化测试
是约束事实源。

| 分类 | 文档 | 重点 |
|---|---|---|
| 初学者学习指南 | [beginner-guide/README.md](beginner-guide/README.md) | 控制面、媒体面、ONVIF、WHEP/HLS、凭据与协调器基础 |
| 工作流程与流程树 | [project-workflow.md](project-workflow.md) | 启动、登录、摄像头变更、播放、录像与发布流程 |
| 完整功能与取舍 | [feature-inventory-and-tradeoffs.md](feature-inventory-and-tradeoffs.md) | 当前能力、Administrator 权限边界、删除后果和明确不做的事项 |
| 必要 README | [../README.md](../README.md) | 项目定位和最短验证入口 |
| [安全与工程边界](unsafe-audit.md) | unsafe 约束、目录职责与真实验证范围 |
| 运维 | [operations.md](operations.md) | 构建、bootstrap、配置、锁、doctor和事件响应 |

当前实现说明：[版本 1.0.0](releases/1.0.0.md)。新 Source 正在完成正式发行验收，已发布旧发行记录保留。

公共支撑的职责、单体依赖、平台边界与验证方法见[公共支撑说明](common-support.md)。
