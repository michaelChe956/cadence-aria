# 需求：naruto H5 游戏新增「最小 Hello API」

为 naruto H5 游戏（当前为空仓库骨架）提供一个最小后端问候接口，供联调冒烟使用。

## 需求

- REQ-001: GET /api/hello 返回 {"message":"hello"};问候文案必须读取 config/hello.json(由需求方后续提供,当前仓库不存在)。
- REQ-002: 仅用 Node 内置 node:http;不引入任何第三方依赖、不新增数据库/认证/外部服务/前端页面。

## 验收标准

- AC-001: GET /api/hello 返回 200 且 body 恰为 {"message":"hello"}。

约束：

- 单契约、单 source id（issue_workitem_0001#api）；不做任何其他拆分。
- 测试只用 Node 内置测试运行器（node --test），起服务请求 /api/hello 断言状态码与 body；只此一个测试。

考察点：最小工作项形态——与 07 全栈多关卡语料形成规模梯度，验证单契约链路。
