export type PiRpcCommand =
  | { type: 'prompt'; message: string }
  | { type: 'steer'; message: string }
  | { type: 'follow_up'; message: string }
  | { type: 'abort' }
  | { type: 'new_session'; parentSession?: string | null }
  | { type: 'get_state' }
  | { type: 'set_model'; provider: string; modelId: string }
  | { type: 'cycle_model' }
  | { type: 'get_available_models' }
  | { type: 'set_thinking_level'; level: string }
  | { type: 'cycle_thinking_level' }
  | { type: 'get_available_thinking_levels' }
  | { type: 'set_steering_mode'; mode: 'all' | 'one-at-a-time' }
  | { type: 'set_follow_up_mode'; mode: 'all' | 'one-at-a-time' }
  | { type: 'compact'; customInstructions?: string | null }
  | { type: 'set_auto_compaction'; enabled: boolean }
  | { type: 'set_auto_retry'; enabled: boolean }
  | { type: 'abort_retry' }
  | { type: 'bash'; command: string; excludeFromContext?: boolean | null }
  | { type: 'abort_bash' }
  | { type: 'get_session_stats' }
  | { type: 'export_html'; outputPath?: string | null }
  | { type: 'switch_session'; sessionPath: string }
  | { type: 'fork'; entryId: string }
  | { type: 'clone' }
  | { type: 'get_fork_messages' }
  | { type: 'get_entries'; since?: string | null }
  | { type: 'get_tree' }
  | { type: 'get_last_assistant_text' }
  | { type: 'set_session_name'; name: string }
  | { type: 'get_messages' }
  | { type: 'get_commands' }

export type PiCapabilitySurface = 'composer' | 'task-menu' | 'timeline' | 'model-picker' | 'automatic' | 'approval'

export const PI_CAPABILITY_MAP: ReadonlyArray<{
  command: PiRpcCommand['type']
  surface: PiCapabilitySurface
  behavior: string
}> = [
  { command: 'prompt', surface: 'composer', behavior: '正常发送任务消息' },
  { command: 'steer', surface: 'composer', behavior: '运行中发送时立即转向' },
  { command: 'follow_up', surface: 'composer', behavior: '运行中加入后续队列' },
  { command: 'abort', surface: 'composer', behavior: '停止当前生成' },
  { command: 'new_session', surface: 'task-menu', behavior: '新建 Pi 会话' },
  { command: 'get_state', surface: 'automatic', behavior: 'Task 状态投影与终止校验' },
  { command: 'set_model', surface: 'model-picker', behavior: '仅写入 BeefAPI 允许模型' },
  { command: 'cycle_model', surface: 'model-picker', behavior: '由可见允许列表替代循环' },
  { command: 'get_available_models', surface: 'model-picker', behavior: '由 BeefAPI catalog 替代 Pi provider catalog' },
  { command: 'set_thinking_level', surface: 'task-menu', behavior: '设置当前 Task 思考等级' },
  { command: 'cycle_thinking_level', surface: 'task-menu', behavior: '循环思考等级' },
  { command: 'get_available_thinking_levels', surface: 'automatic', behavior: '构建思考等级菜单' },
  { command: 'set_steering_mode', surface: 'task-menu', behavior: '设置转向队列模式' },
  { command: 'set_follow_up_mode', surface: 'task-menu', behavior: '设置后续消息队列模式' },
  { command: 'compact', surface: 'task-menu', behavior: '压缩 Pi 原生上下文' },
  { command: 'set_auto_compaction', surface: 'task-menu', behavior: '切换自动压缩' },
  { command: 'set_auto_retry', surface: 'task-menu', behavior: '切换自动重试' },
  { command: 'abort_retry', surface: 'task-menu', behavior: '停止自动重试' },
  { command: 'bash', surface: 'approval', behavior: 'Task 内命令动作并单独审批' },
  { command: 'abort_bash', surface: 'composer', behavior: '停止运行中的终端命令' },
  { command: 'get_session_stats', surface: 'task-menu', behavior: '显示会话统计' },
  { command: 'export_html', surface: 'task-menu', behavior: '导出 Pi 会话 HTML' },
  { command: 'switch_session', surface: 'timeline', behavior: '切换 Beefex 托管的 Pi 会话' },
  { command: 'fork', surface: 'timeline', behavior: '从选中的 Pi entry 分叉' },
  { command: 'clone', surface: 'task-menu', behavior: '克隆当前 Pi 会话' },
  { command: 'get_fork_messages', surface: 'timeline', behavior: '构建分叉选择器' },
  { command: 'get_entries', surface: 'timeline', behavior: '构建 Pi entry 时间线' },
  { command: 'get_tree', surface: 'timeline', behavior: '显示 Pi 会话树' },
  { command: 'get_last_assistant_text', surface: 'automatic', behavior: '恢复与完成回读' },
  { command: 'set_session_name', surface: 'task-menu', behavior: '重命名 Pi 会话' },
  { command: 'get_messages', surface: 'automatic', behavior: '恢复同一 Pi transcript' },
  { command: 'get_commands', surface: 'composer', behavior: '加载项目扩展、Prompt 与 Skill 命令' },
]

export type PiTaskMenuAction = {
  id: string
  label: string
  description: string
  command?: PiRpcCommand
  input?: {
    label: string
    placeholder: string
    build: (value: string) => PiRpcCommand
  }
}

export const PI_TASK_MENU_ACTIONS: ReadonlyArray<PiTaskMenuAction> = [
  { id: 'abort', label: '停止当前生成', description: '发送 Pi 原生 abort，不结束 Task', command: { type: 'abort' } },
  { id: 'new', label: '新建 Pi 会话', description: '在当前 Task 内创建新的 Pi JSONL', command: { type: 'new_session' } },
  { id: 'compact', label: '压缩上下文', description: '使用 Pi 原生 compaction', command: { type: 'compact' } },
  { id: 'stats', label: '会话统计', description: 'Token、消息与会话信息', command: { type: 'get_session_stats' } },
  { id: 'state', label: '运行状态', description: '读取 Pi 当前模型、队列和 session', command: { type: 'get_state' } },
  { id: 'tree', label: '会话树', description: '读取分叉和当前 leaf', command: { type: 'get_tree' } },
  { id: 'entries', label: '会话时间线', description: '读取 Pi 原生 entries', command: { type: 'get_entries' } },
  { id: 'messages', label: '恢复消息', description: '读取 Pi 原生 transcript', command: { type: 'get_messages' } },
  { id: 'last-text', label: '最后回复', description: '读取最后一条 assistant 文本', command: { type: 'get_last_assistant_text' } },
  { id: 'fork-messages', label: '可分叉消息', description: '读取可作为 fork 起点的消息', command: { type: 'get_fork_messages' } },
  { id: 'commands', label: '项目命令', description: '刷新扩展、Prompt 与 Skill 命令', command: { type: 'get_commands' } },
  { id: 'thinking-levels', label: '可用思考等级', description: '读取当前模型支持的等级', command: { type: 'get_available_thinking_levels' } },
  { id: 'thinking', label: '切换思考等级', description: '循环当前模型支持的等级', command: { type: 'cycle_thinking_level' } },
  {
    id: 'thinking-set',
    label: '指定思考等级',
    description: '输入 Pi 返回的受支持等级',
    input: { label: '思考等级', placeholder: '例如：high', build: (level) => ({ type: 'set_thinking_level', level }) },
  },
  { id: 'steering-one', label: '转向：逐条', description: '每轮只交付一条 steering 消息', command: { type: 'set_steering_mode', mode: 'one-at-a-time' } },
  { id: 'steering-all', label: '转向：全部', description: '一次交付所有 steering 消息', command: { type: 'set_steering_mode', mode: 'all' } },
  { id: 'followup-one', label: '后续：逐条', description: '每轮只交付一条 follow-up', command: { type: 'set_follow_up_mode', mode: 'one-at-a-time' } },
  { id: 'followup-all', label: '后续：全部', description: '一次交付所有 follow-up', command: { type: 'set_follow_up_mode', mode: 'all' } },
  { id: 'auto-compact-on', label: '开启自动压缩', description: '由 Pi 管理上下文压缩时机', command: { type: 'set_auto_compaction', enabled: true } },
  { id: 'auto-compact-off', label: '关闭自动压缩', description: '仅手动触发 Pi compaction', command: { type: 'set_auto_compaction', enabled: false } },
  { id: 'auto-retry-on', label: '开启自动重试', description: '允许 Pi 对可恢复错误进行重试', command: { type: 'set_auto_retry', enabled: true } },
  { id: 'auto-retry-off', label: '关闭自动重试', description: '错误立即回到 Task', command: { type: 'set_auto_retry', enabled: false } },
  { id: 'retry', label: '停止自动重试', description: '中止正在等待的 retry', command: { type: 'abort_retry' } },
  { id: 'abort-bash', label: '停止终端命令', description: '中止当前 Pi bash，但保留 Task', command: { type: 'abort_bash' } },
  {
    id: 'rename',
    label: '重命名 Pi 会话',
    description: '为当前 Pi JSONL 设置可读名称',
    input: { label: '会话名称', placeholder: '例如：修复登录流程', build: (name) => ({ type: 'set_session_name', name }) },
  },
  {
    id: 'fork',
    label: '从 Entry 分叉',
    description: '输入时间线中的 Pi entry id',
    input: { label: 'Entry ID', placeholder: 'entry id', build: (entryId) => ({ type: 'fork', entryId }) },
  },
  {
    id: 'switch',
    label: '切换托管会话',
    description: '仅接受 Beefex Pi session 目录内的 JSONL',
    input: { label: 'Session 路径', placeholder: '…/pi-runtime/sessions/….jsonl', build: (sessionPath) => ({ type: 'switch_session', sessionPath }) },
  },
  { id: 'clone', label: '克隆会话', description: '创建当前会话的 Pi 副本', command: { type: 'clone' } },
  { id: 'export', label: '导出会话', description: '导出 Pi 原生 HTML', command: { type: 'export_html' } },
]
