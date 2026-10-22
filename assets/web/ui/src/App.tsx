import { useState } from 'react'
import { Clapperboard, Image, Layers, Library, PanelLeft, SlidersHorizontal } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { FosforaProvider, useFosfora } from '@/fosfora/client'
import { LayersPanel, LibraryPanel, LookPanel, PicturePanel, SectionInfo, ShowPanel } from '@/panels'

const NAV = [
  { id: 'layers', label: 'Layers', icon: Layers },
  { id: 'look', label: 'Look', icon: SlidersHorizontal },
  { id: 'picture', label: 'Picture', icon: Image },
  { id: 'library', label: 'Library', icon: Library },
  { id: 'show', label: 'Show', icon: Clapperboard },
] as const

type Section = (typeof NAV)[number]['id']

const BARS = ['sub_bass', 'bass', 'low_mid', 'mid', 'upper_mid', 'presence', 'brilliance'] as const

function Shell() {
  const { connected, linkError, state, audio, send } = useFosfora()
  const [section, setSection] = useState<Section>('layers')
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem('fosfora-nav') === 'icons')
  const mode = state?.workspace_mode === 'perform' ? 'perform' : 'prepare'
  const save = state?.save_state || 'saved'
  const scene = state?.applied_preset || 'No scene'
  const loading = state?.loading_preset
  const requested = state?.requested_preset
  const waiting = requested && requested !== scene && !loading ? requested : null

  return (
    <div className="flex h-full bg-background text-foreground">
      <aside className={`flex shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground transition-[width] duration-200 ${collapsed ? 'w-14' : 'w-40'}`}>
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
            aria-label={collapsed ? 'Show menu labels' : 'Show icons only'}
            onClick={() => {
              const next = !collapsed
              localStorage.setItem('fosfora-nav', next ? 'icons' : 'open')
              setCollapsed(next)
            }}
          >
            <PanelLeft />
          </Button>
        </div>
        <Separator />
        <nav className={`flex flex-1 flex-col gap-1 ${collapsed ? 'items-center p-1' : 'p-1.5'}`}>
          {NAV.map((item) => {
            const Icon = item.icon
            const on = section === item.id
            const button = (
              <Button
                variant={on ? 'secondary' : 'ghost'}
                size={collapsed ? 'icon' : 'sm'}
                className={collapsed ? 'size-9' : 'w-full justify-start px-2'}
                aria-label={item.label}
                onClick={() => setSection(item.id)}
              >
                <Icon />
                {collapsed ? <span className="sr-only">{item.label}</span> : item.label}
              </Button>
            )
            if (!collapsed) return <div key={item.id}>{button}</div>
            return (
              <Tooltip key={item.id}>
                <TooltipTrigger asChild>{button}</TooltipTrigger>
                <TooltipContent side="right">{item.label}</TooltipContent>
              </Tooltip>
            )
          })}
        </nav>
        <div className={`text-xs text-muted-foreground ${collapsed ? 'flex justify-center p-2' : 'px-3 py-2'}`} title={connected ? 'Connected' : 'Reconnecting'}>
          <span className={`inline-block size-2 rounded-full ${connected ? 'bg-primary' : 'bg-destructive'}`} />
          {collapsed ? null : <span className="ml-2">{connected ? 'Connected' : 'Reconnecting'}</span>}
        </div>
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-2 border-b px-4 py-3">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium">{scene}</div>
            <div className="truncate text-xs text-muted-foreground">
              {loading ? `Loading ${loading}` : waiting ? `Waiting for ${waiting}` : state?.layers?.[state.active_layer]?.name || 'No layer selected'}
            </div>
          </div>
          <div className="hidden items-end gap-1 sm:flex" aria-hidden>
            {BARS.map((band) => (
              <div key={band} className="flex h-8 w-1.5 items-end rounded-full bg-muted">
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
          <Badge variant={save === 'failed' ? 'destructive' : 'secondary'}>
            {save === 'failed' && state?.save_detail ? state.save_detail : save}
          </Badge>
          <Button size="sm" disabled={!!state?.save_blocked} onClick={() => send({ type: 'save_scene' })}>
            Save scene
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
        {state?.load_error ? (
          <div className="border-b px-4 py-2 text-sm text-destructive">Error: {state.load_error}</div>
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
        <main className="flex min-h-0 flex-1 flex-col overflow-hidden p-4">
          {section === 'layers' ? <LayersPanel /> : (
            <div className="min-h-0 flex-1 overflow-auto">
              {section === 'look' ? <LookPanel /> : null}
              {section === 'picture' ? <PicturePanel /> : null}
              {section === 'library' ? <LibraryPanel /> : null}
              {section === 'show' ? <ShowPanel /> : null}
            </div>
          )}
        </main>
        <footer className="truncate border-t px-4 py-2 font-mono text-[11px] text-muted-foreground">
          {connected ? 'connected' : 'reconnecting'}
          {' · '}
          {state?.engine_commit || 'no commit'}
          {' · pid '}
          {state?.engine_pid ?? '—'}
          {' · '}
          {state?.engine_exe || 'no executable'}
          {' · '}
          {state?.show_path || 'no show'}
          {' · applied '}
          {state?.applied_preset || 'none'}
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
      </FosforaProvider>
    </TooltipProvider>
  )
}
