export type ChannelStatus = { kind: 'telegram' | 'slack' | 'discord' | 'signal' | 'whatsapp' | 'imessage'; connected: boolean; handle: string | null; error: string | null }

export type Status = { bot_id: number; provider: string; pair_code: string | null; channels: ChannelStatus[]; error: string | null }

export type Bot = {
  id: number; name: string; persona: string; provider: string; model: string; base_url: string; tools: string
  has_telegram_token: boolean; has_slack: boolean; has_discord: boolean; has_signal: boolean; has_whatsapp: boolean; has_imessage: boolean
  /** Its creature, as you chose it (JSON of Look); empty = the one its id gives it. */
  look?: string
  /** The folder you chose as its workspace, on the machine Mimir runs on; empty = Mimir's own workspaces. */
  workdir?: string
  /** How its file_write asks: '' (as Permissions says), ask, allow or off. */
  file_write?: string
}

export type Rehearsed = { reply: string; would: string[]; looked: string[] }

export type Chat = { chat_id: number; title: string; memories: number; channel: string; guest?: boolean; household?: boolean; family?: boolean; private?: boolean }

export type Kind = 'convention' | 'knowledge' | 'task'

export type Delivery = 'now' | 'digest' | 'log' | 'card'

export type Routine = { id: number; bot_id: number; quiet: boolean; paused: boolean; chat_id: number; cron: string; prompt: string; next_fire: number; delivery: Delivery; once?: boolean; watch?: string; urgent?: string; ends?: number; followup?: boolean; goal_held?: boolean; for_goal?: boolean }

export type Tab = 'chat' | 'memory' | 'skills' | 'routines' | 'activity'

export type ComputerStatus = { available: boolean; enabled: boolean; running: boolean }

export type Run = {
  id: number; bot_id: number; chat_id: number; reply?: string | null; parent_run?: number | null; trigger: 'message' | 'routine' | 'rehearsal' | 'tidy' | 'delegated' | 'reported' | 'import' | 'know_me' | 'check' | 'catch_up'; routine_id: number | null; provider: string; model: string | null
  status: 'running' | 'ok' | 'error' | 'missed' | 'unchanged'; error: string | null; started_at: number; ended_at: number | null
  tokens_in: number; tokens_out: number; cost_usd: number | null; tool_calls: number
  delivered?: string | null; repeats?: number; cache_read?: number; cache_write?: number; estimated?: boolean
}

export type Approval = { id: number; bot_id: number; chat_id: number; tool: string; summary: string; created_at: number; expires_at: number; kind?: 'rule' | 'guarded' | 'guest' | 'login' | 'plan' | 'bot' | 'send' | 'takeover' | 'signin' | 'remind' | 'unasked' | 'loop' | 'sensitive' | 'drift' | 'userfile' | 'target' | 'shell' | 'purchase'; site?: string | null; parked?: boolean; allowed_before?: number
  /** A card whose text you can edit before it goes: its fields (subject and body editable; to, cc and account shown). */
  draft?: Record<string, string> | null
  /** shown:<where> once a card or the app showed it; failed:<error> (or shown:app; card failed:…) if its chat-app card couldn't be sent. */
  delivery?: string | null }

export type Busy = { bot_id: number; chat_id: number; trigger: string; step: string | null }

export type Account = { installed: boolean; signed_in?: boolean; version?: string; plan?: string | null; account?: string | null; key?: boolean; error?: string | null; server?: string; base?: string; models?: Model[] }

export type Model = { id: string; tools: boolean | null; name?: string }

export type Hold = { key: string; bot_id: number; routine_id: number | null; routine?: string | null; person?: string | null; until: number; why: string; cap: boolean }

/** A chat's goal: one lasting objective its bot works on until it's done. */
export type Goal = { id: number; bot_id: number; chat_id: number; text: string; done_when: string; status: 'active' | 'paused' | 'blocked' | 'done'; progress: string; why: string; budget: number; spent: number; tainted: boolean; updated_at: number }

/** A plain ask you keep allowing (5 times in 30 days, never denied): Needs you offers to stop asking. */
export type RuleHint = { tool: string; site: string | null; times: number; bot_id: number }
export type Inbox = { approvals: Approval[]; missed: Approval[]; failed: Run[]; holds?: Hold[]; goals?: Goal[]; suggestions: { chat_id: number; count: number }[]; unconfirmed?: { chat_id: number; count: number }[]; rule_hints?: RuleHint[]; mail?: HandedOver[]; server_update?: { current: string; latest: string; how: string } | null; damage?: string | null
  /** Routines paused since then because you were away; your next message (or Resume) starts them again. */
  away?: number | null }

/** A reply to a bot's email it handed to you instead of answering: who, what, and the bot's one line. */
export type HandedOver = { id: number; bot_id: number; chat_id: number; account: string; from: string; subject: string; summary: string; at: number }

/** A pause: the bot, or null for all of them; until when (0: until you resume). */
export type Pause = { bot_id: number | null; until: number }

export type Skill = {
  id: number; bot_id: number; name: string; description: string; body: string; run_id: number | null; status: 'proposed' | 'active'; created_at: number
  kind: 'learned' | 'patch' | 'merge' | 'retire' | 'taught' | 'saved' | 'library'; reason: string; replaces: number[]; uses: number; last_used: number | null
}

export type LibrarySkill = { slug: string; category: string; name: string; description: string; body: string; needs: string[]; added: number[]; missing: Record<string, string[]>; schedule: { cron: string; label: string; prompt: string } | null }

export type ToolServer = { name: string; preset: string | null; command: string; args: string[]; url: string; env: string[]; headers: string[]; tools: number; signed_in?: boolean; sensitive?: boolean; suggest_sensitive?: boolean; update?: string | null; unconfirmed?: number }
