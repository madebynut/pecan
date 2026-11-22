# 长代码块与底边框验收

保持代码完整展开，先横向滚动查看长行，再上下滚动正文，让代码底部从视口外进入视口。底边框应连续，最后一行与后续正文之间应保留合适留白。可同时调整 Tab、格式化与折叠后再展开，检查边框。

## 插件配置

```json
{
    "reader_setting": {
        "mode": "reading",
        "version": 1,
        "plugins": [
            {
                "id": 1,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 1 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 2,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 2 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 3,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 3 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 4,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 4 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 5,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 5 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 6,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 6 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 7,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 7 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 8,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 8 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 9,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 9 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 10,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 10 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 11,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 11 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 12,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 12 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 13,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 13 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 14,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 14 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 15,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 15 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 16,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 16 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 17,
                "plugin": "mermaid",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 17 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            },
            {
                "id": 18,
                "plugin": "math",
                "enabled": true,
                "options": {
                    "timeout": 30000,
                    "cache": true,
                    "description": "第 18 项用于验收原生代码底边框与滚动，完整复制时应保留。"
                }
            }
        ]
    }
}
```

## 代码后的正文

代码最后应以闭合的花括号结束，底边框完整，没有额外空行。导出时完整保留 18 个插件；全文搜索「第 18 项」可检查末项。
