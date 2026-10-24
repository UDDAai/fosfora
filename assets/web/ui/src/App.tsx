import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { Clapperboard, Image, Layers, Library, PanelLeft, SlidersHorizontal } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Toaster } from '@/components/ui/sonner'
import { Separator } from '@/components/ui/separator'
import { TooltipProvider } from '@/components/ui/tooltip'
import { FosforaProvider, useFosfora } from '@/fosfora/client'
import { clock } from '@/fosfora/types'
import { EffectsPanel, LayersPanel, LibraryPanel, PicturePanel, SectionInfo, ShowPanel } from '@/panels'

const NAV = [
  { id: 'show', label: 'Show', icon: Clapperboard },
  { id: 'layers', label: 'Layers', icon: Layers },
  { id: 'effects', label: 'Effects', icon: SlidersHorizontal },
  { id: 'picture', label: 'Picture', icon: Image },
  { id: 'library', label: 'Library', icon: Library },
] as const

type Section = (typeof NAV)[number]['id']

const BARS = ['sub_bass', 'bass', 'low_mid', 'mid', 'upper_mid', 'presence', 'brilliance'] as const
const NAV_MIN = 144
const NAV_MAX = 320
// Below this window width the menu folds to icons by itself so a half-screen
// browser keeps room for the header and the panels.
const NARROW_QUERY = '(max-width: 799px)'

function pageFromPath(path: string): Section | null {
  const name = path.split('?')[0].replace(/\/+$/, '').replace(/^\//, '')
  if (name === 'look') return 'effects'
  return NAV.some((item) => item.id === name) ? (name as Section) : null
}

function readNavWidth() {
  const raw = localStorage.getItem('fosfora-nav-width')
  const saved = Number(raw)
  if (raw == null || !Number.isFinite(saved)) return 160
  return Math.min(NAV_MAX, Math.max(NAV_MIN, saved))
}

function readNavPref() {
  return localStorage.getItem('fosfora-nav') === 'icons'
}

function Shell() {
  const { connected, linkError, state, audio, send } = useFosfora()
  const [section, setSection] = useState<Section>(() => pageFromPath(window.location.pathname) ?? 'layers')
  const [narrow, setNarrow] = useState(() => window.matchMedia(NARROW_QUERY).matches)
  const [collapsed, setCollapsed] = useState(() => window.matchMedia(NARROW_QUERY).matches || readNavPref())
  const [navWidth, setNavWidth] = useState(readNavWidth)
  const mode = state?.workspace_mode === 'perform' ? 'perform' : 'prepare'

  // Engine errors pop up for 5 s. A message already on screen does not pop
  // again on the next state tick; it can once the engine has dropped it.
  const shownErrors = useRef(new Set<string>())
  const errorMessages = state?.error_messages
  useEffect(() => {
    const now = new Set(errorMessages ?? [])
    for (const message of now) {
      if (!shownErrors.current.has(message)) {
        toast.error(message, { id: message, duration: 5000, closeButton: true })
      }
    }
    shownErrors.current = now
  }, [errorMessages])

  // Crossing the width line folds the menu, or restores the saved choice.
  // A toggle made while narrow lasts until the window widens again.
  useEffect(() => {
    const query = window.matchMedia(NARROW_QUERY)
    const onChange = (event: MediaQueryListEvent) => {
      setNarrow(event.matches)
      setCollapsed(event.matches || readNavPref())
    }
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])

  useEffect(() => {
    const path = window.location.pathname.replace(/\/+$/, '') || '/'
    if (path === '/look') {
      history.replaceState(null, '', '/effects')
    } else if (pageFromPath(path) == null) {
      history.replaceState(null, '', '/layers')
    }
    const onPop = () => setSection(pageFromPath(window.location.pathname) ?? 'layers')
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  function openSection(id: Section) {
    setSection(id)
    const path = `/${id}`
    if (window.location.pathname !== path) history.pushState(null, '', path)
  }

  function resizeNav(event: ReactPointerEvent<HTMLButtonElement>) {
    event.preventDefault()
    const startX = event.clientX
    const startW = navWidth
    function move(next: PointerEvent) {
      setNavWidth(Math.min(NAV_MAX, Math.max(NAV_MIN, startW + next.clientX - startX)))
    }
    function up(next: PointerEvent) {
      const width = Math.min(NAV_MAX, Math.max(NAV_MIN, startW + next.clientX - startX))
      setNavWidth(width)
      localStorage.setItem('fosfora-nav-width', String(Math.round(width)))
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }
  const save = state?.save_state || 'saved'
  const scene = state?.applied_preset || 'No scene'
  const loading = state?.loading_preset
  const requested = state?.requested_preset
  const waiting = requested && requested !== scene && !loading ? requested : null

  return (
    <div className="flex h-full bg-background text-foreground">
      <aside
        className={`relative flex shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground ${collapsed ? 'w-[4.5rem]' : ''}`}
        style={collapsed ? undefined : { width: navWidth }}
      >
        {collapsed ? null : (
          <button
            type="button"
            aria-label="Resize menu"
            className="absolute top-0 right-0 z-10 h-full w-1.5 cursor-col-resize hover:bg-primary/40"
            onPointerDown={resizeNav}
          />
        )}
        <div className={`flex items-center ${collapsed ? 'justify-center px-1 py-3' : 'justify-between px-3 py-3'}`}>
          {collapsed ? null : (
            <div className="min-w-0">
              <div className="text-[10px] tracking-wide text-muted-foreground">Fosfora</div>
              <div className="truncate text-sm font-semibold">Control</div>
            </div>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="size-8"
            aria-label={collapsed ? 'Expand menu' : 'Compact menu'}
            onClick={() => {
              const next = !collapsed
              if (!narrow) localStorage.setItem('fosfora-nav', next ? 'icons' : 'open')
              setCollapsed(next)
            }}
          >
            <PanelLeft />
          </Button>
        </div>
        <Separator />
        <nav className={`flex flex-1 flex-col ${collapsed ? 'gap-3 p-1.5' : 'gap-2 p-1.5'}`}>
          {NAV.map((item) => {
            const Icon = item.icon
            const on = section === item.id
            return (
              <Button
                key={item.id}
                variant={on ? 'secondary' : 'ghost'}
                size={collapsed ? 'default' : 'sm'}
                className={collapsed ? 'h-auto w-full flex-col gap-1 px-1 py-2 text-[10px] leading-none font-normal' : 'w-full justify-start px-2'}
                aria-label={item.label}
                aria-current={on ? 'page' : undefined}
                onClick={() => openSection(item.id)}
              >
                <Icon />
                {item.label}
              </Button>
            )
          })}
        </nav>
        <div className={`text-xs text-muted-foreground ${collapsed ? 'flex justify-center p-2' : 'px-3 py-2'}`} title={connected ? 'Connected' : 'Reconnecting'}>
          <span className={`inline-block size-2 rounded-full ${connected ? 'bg-primary' : 'bg-destructive'}`} />
          {collapsed ? null : <span className="ml-2">{connected ? 'Connected' : 'Reconnecting'}</span>}
        </div>
      </aside>
      <div className="@container flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-1.5 border-b px-3 py-3 @2xl:gap-2 @2xl:px-4">
          <div className="flex min-w-48 flex-1 items-center gap-3">
            <div className="shrink-0 font-mono text-lg tabular-nums tracking-tight">
              {state?.show?.loaded ? clock(state.show.elapsed_ms) : '--:--:--'}
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-medium">{scene}</div>
              <div className="text-[10px] leading-none tracking-wide text-muted-foreground">Scene</div>
            </div>
            {loading || waiting ? (
              <div className="hidden truncate text-xs text-muted-foreground @3xl:block">
                {loading ? `Loading ${loading}` : `Waiting for ${waiting}`}
              </div>
            ) : null}
          </div>
          <div className="hidden items-end gap-1 self-center @4xl:flex" aria-hidden>
            {BARS.map((band) => (
              <div key={band} className="flex h-5 w-1.5 items-end rounded-full bg-muted">
                <div
                  className="w-full rounded-full bg-primary"
                  style={{ height: `${Math.round((audio?.[band] ?? 0) * 100)}%` }}
                />
              </div>
            ))}
          </div>
          <Badge variant="outline">{audio && audio.bpm > 1 ? `${Math.round(audio.bpm)} BPM` : 'BPM'}</Badge>
          <SectionInfo
            label="About Prepare and Perform"
            text="Prepare writes the open scene shortly after you edit. Perform holds those edits until you save them. The name at the left is the scene Fosfora has applied, and the badge next to Save is whether that scene is saved."
          />
          <Button
            size="sm"
            variant={mode === 'prepare' ? 'secondary' : 'ghost'}
            onClick={() => send({ type: 'set_workspace_mode', mode: 'prepare' })}
          >
            Prepare
          </Button>
          <Button
            size="sm"
            variant={mode === 'perform' ? 'secondary' : 'ghost'}
            onClick={() => send({ type: 'set_workspace_mode', mode: 'perform' })}
          >
            Perform
          </Button>
          <Badge
            variant={save === 'failed' ? 'destructive' : 'secondary'}
            className="max-w-28 @4xl:max-w-64"
            title={save === 'failed' && state?.save_detail ? state.save_detail : undefined}
          >
            <span className="truncate">{save === 'failed' && state?.save_detail ? state.save_detail : save}</span>
          </Badge>
          <Button size="sm" aria-label="Save scene" disabled={!!state?.save_blocked} onClick={() => send({ type: 'save_scene' })}>
            Save<span className="hidden @xl:inline"> scene</span>
          </Button>
        </header>
        {state?.pending_switch ? (
          <div className="flex flex-wrap items-center gap-2 border-b bg-amber-950/40 px-4 py-2">
            <span className="text-sm">Unsaved changes before {state.pending_switch}.</span>
            <Button size="sm" onClick={() => send({ type: 'resolve_pending', action: 'save' })}>Save</Button>
            <Button size="sm" variant="outline" onClick={() => send({ type: 'resolve_pending', action: 'discard' })}>Discard</Button>
            <Button size="sm" variant="ghost" onClick={() => send({ type: 'resolve_pending', action: 'cancel' })}>Cancel</Button>
          </div>
        ) : null}
        {linkError ? <div className="border-b px-4 py-2 text-sm text-destructive">{linkError}</div> : null}
        {(state?.command_drops ?? 0) > 0 ? (
          <div className="border-b px-4 py-2 text-sm text-destructive">
            The engine dropped {state?.command_drops} commands because its queue was full.
          </div>
        ) : null}
        {state?.show?.notice && !state.load_error ? (
          <div className="border-b px-4 py-2 text-sm text-muted-foreground">{state.show.notice}</div>
        ) : null}
        <main className="flex min-h-0 flex-1 flex-col overflow-hidden p-3 @2xl:p-4">
          {section === 'layers' ? <LayersPanel /> : (
            <div className="min-h-0 flex-1 overflow-auto">
              {section === 'effects' ? <EffectsPanel /> : null}
              {section === 'picture' ? <PicturePanel /> : null}
              {section === 'library' ? <LibraryPanel /> : null}
              {section === 'show' ? <ShowPanel /> : null}
            </div>
          )}
        </main>
        <footer className="flex gap-1 border-t px-3 py-2 font-mono text-[11px] text-muted-foreground @2xl:px-4">
          {/* Narrow windows drop the long paths first and ellipsize the rest. */}
          <span className="shrink-0">
            {connected ? 'connected' : 'reconnecting'}
            {' · '}
            {state?.engine_commit || 'no commit'}
            {' · pid '}
            {state?.engine_pid ?? '—'}
          </span>
          <span className="hidden min-w-0 truncate @3xl:inline" title={state?.engine_exe || undefined}>
            {' · '}
            {state?.engine_exe || 'no executable'}
          </span>
          <span className="hidden min-w-0 truncate @2xl:inline" title={state?.show_path || undefined}>
            {' · '}
            {state?.show_path || 'no show'}
          </span>
          <span className="min-w-0 truncate">
            {' · applied '}
            {state?.applied_preset || 'none'}
          </span>
        </footer>
      </div>
    </div>
  )
}

export default function App() {
  return (
    <TooltipProvider>
      <FosforaProvider>
        <Shell />
        <Toaster position="top-center" />
      </FosforaProvider>
    </TooltipProvider>
  )
}
