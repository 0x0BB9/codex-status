# 本地任务看板说明

任务看板支持三类本地标注：

- 标记关注：把重要线程置顶并高亮。
- 本地备注：给线程写看板备注，例如“等接口”、“等设计”、“今日处理”。
- 本地分类：按项目、优先级、阶段给线程打标，并支持在看板里分组展示。

这些数据只属于浮窗，不会写回 Codex 线程，也不会改变 Codex 真实状态。

## 存储位置

本地看板数据存储在：

```text
~/.codex/status-floater/thread-board.json
```

如果设置了 `CODEX_HOME`，并且该目录存在，会使用：

```text
$CODEX_HOME/status-floater/thread-board.json
```

## 数据结构

文件按 `threadId` 存储元数据：

```json
{
  "schemaVersion": 1,
  "threads": {
    "thread-id": {
      "pinned": true,
      "note": "等接口",
      "project": "mos-web",
      "priority": "high",
      "stage": "waiting",
      "updatedAtMs": 1782800000000
    }
  }
}
```

## 展示规则

- 关注线程会排在任务列表前面。
- 非关注线程仍按 Codex 的 `updated_at desc` 排序。
- 搜索会匹配标题、预览、路径、备注、项目、优先级和阶段。
- 分组支持：不分组、按项目、按优先级、按阶段。

## 可用分类

优先级：

- 高优先级
- 中优先级
- 低优先级

阶段：

- 待处理
- 进行中
- 等待中
- 已完成

项目是自由文本，可以按实际项目名填写。
