# 阅评说刮削插件 (WASM)

Ting Reader 的阅评说 (ypshuo.com) 刮削插件，基于 WASM 技术实现。

## 编译

本插件使用 Ting Reader 提供的 WASM 宿主函数，不依赖 WASI。推荐使用 `wasm32-unknown-unknown` 目标构建：

```bash
# 添加构建目标
rustup target add wasm32-unknown-unknown

# 编译
cargo build --target wasm32-unknown-unknown --release
```

编译产物位于：`target/wasm32-unknown-unknown/release/ypshuo_scraper.wasm`

## 安装

1. 将 `ypshuo_scraper.wasm` 复制到插件目录
2. 将 `plugin.yml` 复制到同一目录
3. 重启 Ting Reader 即可自动加载

## 搜索参数

插件使用 `metadata_provider` 能力声明搜索字段；运行时仍接受 `query` 作为外部调用兜底：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `title` | 是 | 搜索字段，书名关键字 |
| `query` | 否 | 外部调用兜底字段，未提供 `title` 时使用 |
| `author` | 否 | 作者过滤字段，用于把更匹配的结果排到前面 |
| `narrator` | 否 | 兼容旧调用，阅评说不返回演播者 |
| `page` | 否 | 页码，默认 `1` |

## 能力声明

`plugin.yml` 通过 capability 声明元数据搜索能力：

```yaml
capabilities:
- id: metadata.search
  kind: metadata_provider
  invoke: search
  auto_scrape: true
  search_fields:
    - key: title
      label:
        zh: 书名
        en: Title
      required: true
      type: text
      default_from: book.title
    - key: author
      label:
        zh: 作者
        en: Author
      required: false
      type: text
      default_from: book.author
  result_fields:
    - title
    - author
    - narrator
    - cover_url
    - description
```

## 说明

- 插件通过 Ting Reader 自定义宿主函数与宿主环境交互
- 使用 HTTP 请求获取阅评说网站数据
