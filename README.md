# 图书馆管理系统 (Library Management System)

Rust 全栈图书馆管理系统：axum 后端 + SQLite 数据库 + 原生前端。

## 功能

- **仪表盘**：馆藏统计、会员数、当前借阅、逾期数
- **图书管理**：增删改查、搜索
- **会员管理**：注册、删除
- **借阅管理**：借书、还书、借阅记录

## 技术栈

| 层 | 技术 |
|---|---|
| 后端 | Rust + axum 0.8 |
| 数据库 | SQLite (sqlx) |
| 前端 | 原生 HTML/CSS/JS (单页应用) |
| 运行时 | Tokio |

## 快速开始

```bash
cargo build
cargo run
```

访问 http://localhost:3001

## API 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/stats` | 统计信息 |
| GET/POST | `/api/books` | 图书列表/添加 |
| GET/PUT/DELETE | `/api/books/{id}` | 图书详情/更新/删除 |
| GET/POST | `/api/members` | 会员列表/添加 |
| GET/DELETE | `/api/members/{id}` | 会员详情/删除 |
| POST | `/api/borrow` | 借书 `{book_id, member_id, days?}` |
| POST | `/api/return` | 还书 `{record_id}` |
| GET | `/api/records` | 借阅记录 |

## 数据库

SQLite 文件 `library.db` 自动创建于项目根目录。三张表：

- `books` — 图书（标题、作者、ISBN、分类、年份、库存）
- `members` — 会员（姓名、邮箱、电话、状态）
- `borrow_records` — 借阅记录（图书ID、会员ID、借期、还期、状态）

借书/还书在事务中执行，自动更新库存。
