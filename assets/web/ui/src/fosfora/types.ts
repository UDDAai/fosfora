export type ParamInfo = {
  name: string
  type: string
  value: number
  authored?: number
  control?: 'authored' | 'audio' | 'palette' | string
  min?: number
  max?: number
}

export type LayerState = {
  index: number
  name: string
  effect_name?: string | null
  blend_mode: number
  blend_name: string
  opacity: number
  enabled: boolean
  locked: boolean
  pinned: boolean
  media_name?: string | null
  media_fit?: string | null
  media_width?: number | null
  media_pos_x?: number | null
  media_pos_y?: number | null
  media_is_video?: boolean
  media_speed?: number | null
  media_looping?: boolean | null
  params: ParamInfo[]
}

export type EffectInfo = { index: number; name: string }
export type PresetInfo = { index: number; name: string; builtin: boolean }

export type LibraryItem = {
  id: string
  name: string
  kind?: string
  bytes?: number
  width?: number
  height?: number
  fps?: number
  duration_secs?: number
  path?: string
  location?: string
  status?: string
  detail?: string
  scenes?: string[]
  thumb?: string
}

export type ShowSnapshot = {
  loaded: boolean
  auto_enabled: boolean
  running: boolean
  elapsed_ms: number
  scene?: string | null
  next_scene?: string | null
  palette?: string | null
  next_palette?: string | null
  next_scene_secs?: number | null
  palette_manual: boolean
  palettes: string[]
  palette_colors?: { id: string; colors: number[][] }[]
  transport?: 'idle' | 'running' | 'paused' | string
  blackout: boolean
  validation: string
  grade_enabled: boolean
  grade_strength: number
  notice: string
}

export type FullState = {
  type: 'state'
  effects: EffectInfo[]
  layers: LayerState[]
  active_layer: number
  presets: PresetInfo[]
  current_preset?: number | null
  postprocess_enabled: boolean
  library: LibraryItem[]
  library_warnings?: string[]
  save_state: string
  save_detail: string
  workspace_mode: string
  solo_layer: number | null
  pending_switch?: string | null
  save_blocked: boolean
  engine_commit?: string
  engine_pid?: number
  engine_exe?: string
  show_path?: string
  applied_preset?: string | null
  requested_preset?: string | null
  loading_preset?: string | null
  load_error?: string | null
  error_messages?: string[]
  command_drops?: number
  show?: ShowSnapshot | null
}

export type AudioState = {
  type: 'audio'
  sub_bass: number
  bass: number
  low_mid: number
  mid: number
  upper_mid: number
  presence: number
  brilliance: number
  rms: number
  beat: number
  bpm: number
}

export const BLENDS = [
  'Normal',
  'Add',
  'Screen',
  'Color Dodge',
  'Multiply',
  'Overlay',
  'Hard Light',
  'Difference',
  'Exclusion',
  'Subtract',
  'Displace',
  'Refract',
  'Lens',
] as const

export function clock(ms: number) {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  return [h, m, s].map((n) => String(n).padStart(2, '0')).join(':')
}
