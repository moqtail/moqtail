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

import { ReactNode, useLayoutEffect, useRef, useState } from 'react';
import {
  ArrowDownLeft,
  ArrowUpRight,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Trash2,
  X,
} from 'lucide-react';
import { LogEntry, LogEntryKind, formatTime } from './moq/describe';
import { DataStats } from './moq/dataStats';
import { DataStatsPanel } from './DataStatsPanel';

type MessageLogPaneProps = {
  isOpen: boolean;
  entries: LogEntry[];
  dataStats: DataStats;
  onClear: () => void;
  onResetData: () => void;
  onClose: () => void;
};

type Tab = 'messages' | 'data';

/** How close to the bottom, in px, still counts as following the newest message. */
const FOLLOW_THRESHOLD = 24;
const ALL_TYPES = '';

const KIND_LABEL: Record<LogEntryKind, string> = { control: 'Control', signal: 'Signalling' };

function Toggle({
  isOn,
  onClick,
  children,
}: {
  isOn: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      aria-pressed={isOn}
      className={`rounded-full px-2.5 py-1 text-[11px] font-medium ring-1 transition-colors ${
        isOn
          ? 'bg-white/10 text-neutral-100 ring-white/15'
          : 'text-neutral-500 ring-white/8 hover:text-neutral-300'
      }`}
    >
      {children}
    </button>
  );
}

/**
 * MOQT control messages and the signalling that rides on the shared signalling track,
 * interleaved in arrival order so a signal can be read next to the MOQT traffic it causes.
 */
export function MessageLogPane({
  isOpen,
  entries,
  dataStats,
  onClear,
  onResetData,
  onClose,
}: MessageLogPaneProps) {
  const [tab, setTab] = useState<Tab>('messages');
  const [shownKinds, setShownKinds] = useState<Record<LogEntryKind, boolean>>({
    control: true,
    signal: true,
  });
  const [showEchoes, setShowEchoes] = useState(true);
  const [typeFilter, setTypeFilter] = useState(ALL_TYPES);
  const [textFilter, setTextFilter] = useState('');
  // Each row is expanded when it differs from the default: "expand all" and "collapse all"
  // set the default and drop every per-row choice.
  const [isExpandedByDefault, setIsExpandedByDefault] = useState(false);
  const [toggledIds, setToggledIds] = useState<ReadonlySet<number>>(new Set());
  const listRef = useRef<HTMLDivElement>(null);
  const isFollowingRef = useRef(true);

  const counts = { control: 0, signal: 0 };
  const typesByKind: Record<LogEntryKind, Set<string>> = { control: new Set(), signal: new Set() };
  for (const e of entries) {
    counts[e.kind]++;
    typesByKind[e.kind].add(e.name);
  }

  const needle = textFilter.trim().toLowerCase();
  const visible = entries.filter(
    e =>
      shownKinds[e.kind] &&
      (showEchoes || !e.isEcho) &&
      (typeFilter === ALL_TYPES || e.name === typeFilter) &&
      (!needle ||
        e.name.toLowerCase().includes(needle) ||
        e.fields.some(([, v]) => v.toLowerCase().includes(needle))),
  );

  const isExpanded = (id: number) => isExpandedByDefault !== toggledIds.has(id);
  const isAllExpanded = visible.length > 0 && visible.every(e => isExpanded(e.id));

  function toggleEntry(id: number) {
    setToggledIds(prev => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  function setAllExpanded(expanded: boolean) {
    setIsExpandedByDefault(expanded);
    setToggledIds(new Set());
  }

  // Keep the newest message in view unless the user has scrolled up to read older ones.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (list && isOpen && tab === 'messages' && isFollowingRef.current)
      list.scrollTop = list.scrollHeight;
  }, [visible.length, isOpen, tab]);

  function toggleKind(kind: LogEntryKind) {
    setShownKinds(prev => ({ ...prev, [kind]: !prev[kind] }));
    // A type from a kind that is now hidden would leave the list empty.
    if (typesByKind[kind].has(typeFilter)) setTypeFilter(ALL_TYPES);
  }

  return (
    <aside
      className={`w-[26rem] max-w-full shrink-0 flex-col border-l border-white/6 bg-neutral-950 ${
        isOpen ? 'flex' : 'hidden'
      }`}
    >
      <div className="flex h-11 shrink-0 items-center justify-between gap-2 border-b border-white/6 px-3">
        <div role="tablist" className="flex items-center gap-1">
          {(['messages', 'data'] as const).map(t => (
            <button
              key={t}
              role="tab"
              aria-selected={tab === t}
              onClick={() => setTab(t)}
              className={`rounded-md px-2 py-1 text-xs font-semibold transition-colors ${
                tab === t
                  ? 'bg-white/10 text-neutral-100'
                  : 'text-neutral-500 hover:text-neutral-300'
              }`}
            >
              {t === 'messages' ? 'Messages' : 'Data'}
              {t === 'messages' && (
                <span className="ml-1.5 font-normal text-neutral-500">
                  {visible.length === entries.length
                    ? entries.length
                    : `${visible.length} of ${entries.length}`}
                </span>
              )}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1">
          {tab === 'messages' && (
            <button
              onClick={() => setAllExpanded(!isAllExpanded)}
              aria-label={isAllExpanded ? 'Collapse all' : 'Expand all'}
              title={isAllExpanded ? 'Collapse all' : 'Expand all'}
              className="rounded p-1.5 text-neutral-500 transition-colors hover:bg-white/5 hover:text-neutral-200"
            >
              {isAllExpanded ? (
                <ChevronsDownUp className="h-3.5 w-3.5" />
              ) : (
                <ChevronsUpDown className="h-3.5 w-3.5" />
              )}
            </button>
          )}
          <button
            onClick={tab === 'messages' ? onClear : onResetData}
            aria-label={tab === 'messages' ? 'Clear messages' : 'Reset counters'}
            title={tab === 'messages' ? 'Clear messages' : 'Reset counters'}
            className="rounded p-1.5 text-neutral-500 transition-colors hover:bg-white/5 hover:text-neutral-200"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={onClose}
            aria-label="Close messages"
            title="Close"
            className="rounded p-1.5 text-neutral-500 transition-colors hover:bg-white/5 hover:text-neutral-200"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      <div className={`min-h-0 flex-1 flex-col ${tab === 'messages' ? 'flex' : 'hidden'}`}>
        <div className="flex shrink-0 flex-col gap-2 border-b border-white/6 px-3 py-2">
          <div className="flex flex-wrap items-center gap-1.5">
            {(['control', 'signal'] as const).map(kind => (
              <Toggle key={kind} isOn={shownKinds[kind]} onClick={() => toggleKind(kind)}>
                {KIND_LABEL[kind]} <span className="text-neutral-500">{counts[kind]}</span>
              </Toggle>
            ))}
            <Toggle isOn={showEchoes} onClick={() => setShowEchoes(on => !on)}>
              Own echoes
            </Toggle>
          </div>
          <div className="flex gap-2">
            <select
              value={typeFilter}
              onChange={e => setTypeFilter(e.target.value)}
              aria-label="Message type"
              className="min-w-0 shrink-0 rounded-md border border-white/8 bg-neutral-800/60 px-2 py-1.5 text-xs text-neutral-100 outline-none focus:border-white/20 focus:ring-1 focus:ring-white/10"
            >
              <option value={ALL_TYPES}>All types</option>
              {(['control', 'signal'] as const).map(
                kind =>
                  shownKinds[kind] &&
                  typesByKind[kind].size > 0 && (
                    <optgroup key={kind} label={KIND_LABEL[kind]}>
                      {[...typesByKind[kind]].sort().map(name => (
                        <option key={name} value={name}>
                          {name}
                        </option>
                      ))}
                    </optgroup>
                  ),
              )}
            </select>
            <input
              type="text"
              value={textFilter}
              placeholder="Filter by name or value"
              spellCheck={false}
              onChange={e => setTextFilter(e.target.value)}
              className="min-w-0 flex-1 rounded-md border border-white/8 bg-neutral-800/60 px-2.5 py-1.5 text-xs text-neutral-100 placeholder-neutral-600 outline-none focus:border-white/20 focus:ring-1 focus:ring-white/10"
            />
          </div>
        </div>

        <div
          ref={listRef}
          onScroll={e => {
            const el = e.currentTarget;
            isFollowingRef.current =
              el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD;
          }}
          className="min-h-0 flex-1 overflow-y-auto font-mono text-[11px]"
        >
          {visible.length === 0 ? (
            <p className="px-3 py-6 text-center font-sans text-xs text-neutral-600">
              {entries.length === 0 ? 'No messages yet.' : 'No messages match the filters.'}
            </p>
          ) : (
            visible.map(entry => {
              const isSignal = entry.kind === 'signal';
              const expanded = isExpanded(entry.id);
              return (
                <div
                  key={entry.id}
                  className={`border-b border-white/4 ${isSignal ? 'bg-amber-500/[0.04]' : ''} ${
                    entry.isEcho ? 'opacity-50' : ''
                  }`}
                >
                  <button
                    onClick={() => toggleEntry(entry.id)}
                    aria-expanded={expanded}
                    className={`flex w-full min-w-0 items-center gap-2 px-3 text-left transition-colors hover:bg-white/[0.03] ${
                      expanded ? 'pt-2 pb-1' : 'py-1'
                    }`}
                  >
                    <ChevronRight
                      className={`h-3 w-3 shrink-0 text-neutral-600 transition-transform ${
                        expanded ? 'rotate-90' : ''
                      }`}
                      aria-hidden="true"
                    />
                    <span className="shrink-0 text-neutral-500 tabular-nums">
                      {formatTime(entry.at)}
                    </span>
                    {entry.direction === 'sent' ? (
                      <ArrowUpRight
                        className="h-3.5 w-3.5 shrink-0 text-blue-400"
                        aria-label="Sent"
                      />
                    ) : (
                      <ArrowDownLeft
                        className="h-3.5 w-3.5 shrink-0 text-emerald-400"
                        aria-label="Received"
                      />
                    )}
                    <span
                      className={`shrink-0 font-semibold ${
                        isSignal
                          ? 'text-amber-300'
                          : entry.direction === 'sent'
                            ? 'text-blue-300'
                            : 'text-emerald-300'
                      }`}
                    >
                      {entry.name}
                    </span>
                    {isSignal && (
                      <span className="shrink-0 rounded bg-amber-500/15 px-1 font-sans text-[10px] text-amber-400">
                        signal
                      </span>
                    )}
                    {entry.isEcho && (
                      <span className="shrink-0 rounded bg-white/8 px-1 font-sans text-[10px] text-neutral-400">
                        echo
                      </span>
                    )}
                    {!expanded && entry.fields.length > 0 && (
                      <span className="min-w-0 truncate text-neutral-500">
                        {entry.fields.map(([key, value]) => `${key}=${value}`).join('  ')}
                      </span>
                    )}
                  </button>
                  {expanded && entry.fields.length > 0 && (
                    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 pr-3 pb-2 pl-[6.5rem]">
                      {entry.fields.map(([key, value]) => (
                        <div key={key} className="contents">
                          <dt className="text-neutral-500">{key}</dt>
                          <dd className="break-all text-neutral-300">{value}</dd>
                        </div>
                      ))}
                    </dl>
                  )}
                </div>
              );
            })
          )}
        </div>
      </div>

      <div className={`min-h-0 flex-1 flex-col ${tab === 'data' ? 'flex' : 'hidden'}`}>
        <DataStatsPanel stats={dataStats} isActive={isOpen && tab === 'data'} />
      </div>
    </aside>
  );
}
