/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/**
 * The debugging views behind the "Advanced" toggle. Everything here listens only while
 * the panel is mounted, through public API: element and player events, the player's
 * client for data events, and the catalog element.
 */

import { Fragment, type ComponentChildren, type RefObject } from 'preact';
import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import type { DataEvent } from 'moqtail';
import type { CatalogElement, MOQtailPlayer, PlayerEvents } from 'moqtail/player';
import type { MOQtailVideoElement } from 'moqtail/player/component';
import { followData } from '@/lib/data';
import { cn } from '@/lib/utils';

type Tab = 'events' | 'objects' | 'status' | 'catalog';
type Tone = 'element' | 'player' | 'warn';
type Kind = DataEvent['kind'];

interface EventLine {
  at: number;
  tone: Tone;
  text: string;
}

interface Column {
  lines: string[];
  dirty: boolean;
  pre?: HTMLPreElement | null;
}

interface Props {
  videoRef: RefObject<MOQtailVideoElement>;
  player: MOQtailPlayer | undefined;
  catalog: CatalogElement | undefined;
  session: number;
}

const TABS: { id: Tab; label: string }[] = [
  { id: 'events', label: 'Events' },
  { id: 'objects', label: 'Objects' },
  { id: 'status', label: 'Status' },
  { id: 'catalog', label: 'Catalog' },
];
const MAX_EVENTS = 500;
const MAX_LINES = 1000;
const KINDS: Kind[] = ['header', 'object', 'end'];
const ELEMENT_EVENTS = [
  ...['loadedmetadata', 'durationchange', 'playing', 'pause', 'waiting', 'stalled'],
  ...['seeking', 'seeked', 'ratechange', 'ended', 'streamtypechange', 'error'],
];
const RENDITION_EVENTS = ['addrendition', 'removerendition', 'change'];
const READY_STATES = ['nothing', 'metadata', 'current data', 'future data', 'enough data'];
const TONES: Record<Tone, string> = {
  element: 'text-neutral-300',
  player: 'text-blue-300',
  warn: 'text-amber-300',
};

export function Advanced({ videoRef, player, catalog, session }: Props) {
  const [tab, setTab] = useState<Tab>('events');
  const events = useEvents(videoRef, player, session);
  const objects = useObjects(player, session);

  return (
    <section className="hidden h-[42%] min-h-60 shrink-0 flex-col border-t border-white/6 bg-neutral-950 md:flex">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-white/6 px-3">
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            type="button"
            onClick={() => setTab(id)}
            className={cn(
              'cursor-pointer rounded-md px-2.5 py-1 text-xs font-semibold transition-colors',
              tab === id
                ? 'bg-white/8 text-neutral-100'
                : 'text-neutral-500 hover:text-neutral-300',
            )}
          >
            {label}
          </button>
        ))}
        <div className="ml-auto flex items-center gap-3">
          {tab === 'objects' && (
            <>
              {KINDS.map(kind => (
                <Toggle
                  key={kind}
                  label={kind}
                  on={objects.filters[kind]}
                  onChange={on => objects.setFilters({ ...objects.filters, [kind]: on })}
                />
              ))}
              <Toggle
                label="hide catalog"
                on={objects.filters.hideCatalog}
                onChange={on => objects.setFilters({ ...objects.filters, hideCatalog: on })}
              />
            </>
          )}
          {(tab === 'events' || tab === 'objects') && (
            <button
              type="button"
              onClick={tab === 'events' ? events.clear : objects.clear}
              className="cursor-pointer rounded-md px-2 py-1 text-xs text-neutral-500 transition-colors hover:bg-white/6 hover:text-neutral-200"
            >
              Clear
            </button>
          )}
        </div>
      </div>
      <div className="min-h-0 flex-1">
        {tab === 'events' && <EventsView lines={events.lines} />}
        {tab === 'objects' && <ObjectsView names={objects.names} columns={objects.columns} />}
        {tab === 'status' && <StatusView videoRef={videoRef} player={player} catalog={catalog} />}
        {tab === 'catalog' && <CatalogView catalog={catalog} />}
      </div>
    </section>
  );
}

/** Element, rendition, and player events, newest last. */
function useEvents(
  videoRef: RefObject<MOQtailVideoElement>,
  player: MOQtailPlayer | undefined,
  session: number,
) {
  const [lines, setLines] = useState<EventLine[]>([]);
  // When the last rendition selection changed, to time the switch it asked for
  const switchAt = useRef<number>();

  // A layout effect, so the log is empty before the new source's first event fires
  useLayoutEffect(() => {
    setLines([]);
    switchAt.current = undefined;
  }, [session]);
  const log = (text: string, tone: Tone) =>
    setLines(previous => [
      ...previous.slice(1 - MAX_EVENTS),
      { at: performance.now(), tone, text },
    ]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const abort = new AbortController();
    const signal = abort.signal;
    for (const name of ELEMENT_EVENTS) {
      video.addEventListener(
        name,
        () => log(describeElementEvent(video, name), name === 'error' ? 'warn' : 'element'),
        {
          signal,
        },
      );
    }
    const list = video.videoRenditions;
    let selectedIndex = list.selectedIndex;
    for (const name of RENDITION_EVENTS) {
      list.addEventListener(
        name,
        () => {
          // Only a new selection is timed: the element also fires `change` as renditions load
          if (list.selectedIndex !== selectedIndex) switchAt.current = performance.now();
          selectedIndex = list.selectedIndex;
          log(`videoRenditions ${name}, selectedIndex ${selectedIndex}`, 'element');
        },
        { signal },
      );
    }
    return () => abort.abort();
  }, [videoRef]);

  useEffect(() => {
    if (!player) return;
    const listeners: Partial<PlayerEvents> = {
      statechange: (state, previous) => log(`state ${previous} → ${state}`, 'player'),
      renditionchange: (_source, rendition) => {
        const since = switchAt.current;
        switchAt.current = undefined;
        const took =
          since === undefined
            ? ''
            : `, ${Math.round(performance.now() - since)} ms after the selection`;
        log(
          `renditionchange ${rendition.id} ${size(rendition)} ${kbps(rendition.bitrate)}${took}`,
          'player',
        );
      },
      warning: warning => log(`warning ${warning.code}: ${warning.message}`, 'warn'),
      reconnecting: attempt => log(`reconnecting, attempt ${attempt}`, 'warn'),
      reconnected: () => log('reconnected', 'player'),
      autoplayblocked: () => log('autoplay blocked', 'warn'),
    };
    const entries = Object.entries(listeners) as [keyof PlayerEvents, never][];
    for (const [name, listener] of entries) player.on(name, listener);
    return () => {
      for (const [name, listener] of entries) player.off(name, listener);
    };
  }, [player]);

  return { lines, clear: () => setLines([]) };
}

/** Data events per track, kept outside state and painted once a frame: they arrive per object. */
function useObjects(player: MOQtailPlayer | undefined, session: number) {
  const [names, setNames] = useState<string[]>([]);
  const [filters, setFilters] = useState({
    header: true,
    object: true,
    end: true,
    hideCatalog: true,
  });
  const columns = useRef(new Map<string, Column>());
  const filtersRef = useRef(filters);
  filtersRef.current = filters;

  useEffect(() => {
    if (!player) return;
    // Lines of each stream whose track isn't known yet
    const pending = new Map<number, string[]>();

    const append = (track: string, lines: string[]) => {
      let column = columns.current.get(track);
      if (!column) {
        column = { lines: [], dirty: false };
        columns.current.set(track, column);
        setNames(previous => [...previous, track]);
      }
      column.lines.push(...lines);
      column.dirty = true;
    };

    return followData(player, (event, track) => {
      const line = filtersRef.current[event.kind] ? describeDataEvent(event) : undefined;
      const lines = pending.get(event.streamId) ?? [];
      if (line) lines.push(line);
      if (track === undefined) {
        if (event.kind === 'end') pending.delete(event.streamId);
        else pending.set(event.streamId, lines);
        return;
      }
      pending.delete(event.streamId);
      if (filtersRef.current.hideCatalog && track === 'catalog') return;
      if (lines.length > 0) append(track, lines);
    });
  }, [player]);

  // A new source's tracks get their own columns
  useLayoutEffect(() => {
    columns.current.clear();
    setNames([]);
  }, [session]);

  useEffect(() => {
    let frame = requestAnimationFrame(function paint() {
      for (const column of columns.current.values()) {
        if (column.lines.length > MAX_LINES)
          column.lines.splice(0, column.lines.length - MAX_LINES);
        const pre = column.pre;
        if (!column.dirty || !pre || isSelecting(pre)) continue;
        const atBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 20;
        column.dirty = false;
        pre.textContent = column.lines.join('\n');
        if (atBottom) pre.scrollTop = pre.scrollHeight;
      }
      frame = requestAnimationFrame(paint);
    });
    return () => cancelAnimationFrame(frame);
  }, []);

  const clear = () => {
    columns.current.clear();
    setNames([]);
  };
  return { names, columns: columns.current, filters, setFilters, clear };
}

function EventsView({ lines }: { lines: EventLine[] }) {
  const scroller = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  if (lines.length === 0)
    return <Empty title="No events yet">Element and player events show here as they happen.</Empty>;
  return (
    <div
      ref={scroller}
      onScroll={e => {
        const el = e.currentTarget;
        atBottom.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 20;
      }}
      className="h-full overflow-auto px-4 py-2 font-mono text-[11px] leading-5"
    >
      {lines.map((line, i) => (
        <div key={i} className="flex gap-3">
          <span className="shrink-0 text-neutral-600 tabular-nums">{clock(line.at)}</span>
          <span className={TONES[line.tone]}>{line.text}</span>
        </div>
      ))}
    </div>
  );
}

function ObjectsView({ names, columns }: { names: string[]; columns: Map<string, Column> }) {
  if (names.length === 0)
    return <Empty title="No data streams yet">Objects show here per track as they arrive.</Empty>;
  return (
    <div className="flex h-full gap-px overflow-x-auto bg-white/6">
      {names.map(name => (
        <div key={name} className="flex w-72 shrink-0 flex-col bg-neutral-950">
          <h4 className="shrink-0 truncate border-b border-white/6 px-3 py-1.5 font-mono text-[11px] font-semibold text-neutral-300">
            {name}
          </h4>
          <pre
            ref={pre => {
              const column = columns.get(name);
              if (!column) return;
              column.pre = pre;
              column.dirty = true; // a remounted <pre> starts empty
            }}
            className="min-h-0 flex-1 overflow-auto px-3 py-1.5 font-mono text-[11px] leading-5 text-neutral-400"
          />
        </div>
      ))}
    </div>
  );
}

function StatusView({ videoRef, player, catalog }: Omit<Props, 'session'>) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick(tick => tick + 1), 250);
    return () => clearInterval(id);
  }, []);

  const video = videoRef.current;
  if (!video || !player)
    return <Empty title="Not connected">Connect to a stream to see its status.</Empty>;
  const rows: [string, string][] = [
    ['State', `${player.state} · ${video.streamType}`],
    ['Playhead', `${seconds(video.currentTime)} / ${seconds(video.duration)}`],
    ['Rate', rate(video.playbackRate)],
    ['Ready state', `${video.readyState} (${READY_STATES[video.readyState]})`],
    ['Video', video.videoWidth ? `${video.videoWidth}×${video.videoHeight}` : '–'],
    ['Buffered', ranges(video.buffered)],
    ['Forward buffer', seconds(forward(video))],
    ['Seekable', ranges(video.seekable)],
    ['Live edge start', seconds(video.liveEdgeStart)],
  ];
  const tracks = catalog?.tracks ?? [];
  return (
    <div className="flex h-full gap-8 overflow-auto px-4 py-3">
      <dl className="grid shrink-0 grid-cols-[auto_1fr] content-start gap-x-6 gap-y-1.5 text-xs">
        {rows.map(([label, value]) => (
          <Fragment key={label}>
            <dt className="text-neutral-500">{label}</dt>
            <dd className="font-mono text-[11px] text-neutral-200 tabular-nums">{value}</dd>
          </Fragment>
        ))}
      </dl>
      {tracks.length > 0 && (
        <table className="h-fit text-left text-xs">
          <thead className="text-neutral-500">
            <tr>
              {['Track', 'Role', 'Size', 'Bitrate', 'Keyframe spacing'].map(h => (
                <th key={h} className="pr-6 pb-1.5 font-normal">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="font-mono text-[11px] text-neutral-200 tabular-nums">
            {tracks.map(t => (
              <tr key={t.name}>
                <td className="py-0.5 pr-6">{t.name}</td>
                <td className="pr-6">{t.role ?? '–'}</td>
                <td className="pr-6">{t.width ? size(t) : '–'}</td>
                <td className="pr-6">{t.bitrate ? kbps(t.bitrate) : '–'}</td>
                <td>{t.keyframeSpacing ?? '–'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function CatalogView({ catalog }: { catalog: CatalogElement | undefined }) {
  const json = catalog?.json;
  if (!json)
    return (
      <Empty title="No catalog yet">
        The catalog shows here once it arrives, with every update applied.
      </Empty>
    );
  return (
    <pre className="h-full overflow-auto px-4 py-3 font-mono text-[11px] leading-5 text-neutral-300">
      {JSON.stringify(json, null, 2)}
    </pre>
  );
}

function Toggle({
  label,
  on,
  onChange,
}: {
  label: string;
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-1.5 text-xs text-neutral-400 select-none hover:text-neutral-200">
      <input
        type="checkbox"
        checked={on}
        onChange={e => onChange(e.currentTarget.checked)}
        className="h-3 w-3 cursor-pointer accent-blue-500"
      />
      {label}
    </label>
  );
}

function Empty({ title, children }: { title: string; children: ComponentChildren }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1 text-center select-none">
      <p className="text-xs font-medium text-neutral-400">{title}</p>
      <p className="text-[11px] text-neutral-600">{children}</p>
    </div>
  );
}

function describeElementEvent(video: MOQtailVideoElement, name: string): string {
  if (name === 'streamtypechange') return `streamtypechange ${video.streamType}`;
  if (name === 'ratechange') return `ratechange ${rate(video.playbackRate)}`;
  if (name === 'durationchange') return `durationchange ${seconds(video.duration)}`;
  if (name === 'seeking') return `seeking to ${seconds(video.currentTime)}`;
  if (name === 'error') return `error ${video.error?.code}: ${video.error?.message}`;
  return name;
}

function describeDataEvent(event: DataEvent): string {
  const prefix = `${clock(event.at)} s${event.streamId}`;
  if (event.kind === 'header') {
    const where = event.stream === 'fetch' ? `req ${event.requestId}` : `g${event.groupId}`;
    return `${prefix} HEADER ${event.stream} ${where}`;
  }
  if (event.kind === 'object') {
    const status = event.objectStatus === undefined ? '' : ` status ${event.objectStatus}`;
    return `${prefix} ${event.groupId}.${event.objectId} ${event.payloadLength} B${status}`;
  }
  const code = event.resetCode === undefined ? '' : ` ${event.resetCode}`;
  return `${prefix} END ${event.reason}${code}`;
}

/** Local wall-clock time of a `performance.now()` stamp, to the millisecond. */
function clock(at: number): string {
  const date = new Date(performance.timeOrigin + at);
  const pad = (n: number, width = 2) => String(n).padStart(width, '0');
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`;
}

function seconds(value: number): string {
  if (Number.isNaN(value)) return '–';
  return Number.isFinite(value) ? `${value.toFixed(2)} s` : 'live';
}

function rate(value: number): string {
  return `${Number(value.toFixed(2))}×`;
}

/** How far the playhead is behind the end of the buffered range it's in; `NaN` outside one. */
function forward(video: HTMLVideoElement): number {
  const { buffered, currentTime } = video;
  for (let i = 0; i < buffered.length; i++) {
    if (buffered.start(i) <= currentTime && currentTime <= buffered.end(i))
      return buffered.end(i) - currentTime;
  }
  return NaN;
}

function ranges(list: TimeRanges): string {
  const parts = Array.from(
    { length: list.length },
    (_, i) => `${list.start(i).toFixed(2)}–${list.end(i).toFixed(2)}`,
  );
  return parts.join('  ') || '–';
}

function size(r: { width?: number; height?: number }): string {
  return `${r.width ?? '?'}×${r.height ?? '?'}`;
}

function kbps(bitrate: number): string {
  return `${Math.round(bitrate / 1000)} kb/s`;
}

function isSelecting(pre: HTMLPreElement): boolean {
  const selection = document.getSelection();
  return !!selection && !selection.isCollapsed && selection.containsNode(pre, true);
}
