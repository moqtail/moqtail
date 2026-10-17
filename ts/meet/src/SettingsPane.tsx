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

import { useEffect, useState } from 'react';
import { X } from 'lucide-react';

type SettingsPaneProps = {
  relay: string;
  defaultRelay: string;
  onSave: (relay: string) => void;
  onClose: () => void;
};

export function SettingsPane({ relay, defaultRelay, onSave, onClose }: SettingsPaneProps) {
  const [relayDraft, setRelayDraft] = useState(relay);
  const isRelayEmpty = !relayDraft.trim();

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  function save() {
    if (isRelayEmpty) return;
    // Accept a pasted full URL; the scheme is fixed and shown beside the field.
    onSave(relayDraft.trim().replace(/^[a-z][a-z0-9+.-]*:\/\//i, ''));
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 px-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        onClick={e => e.stopPropagation()}
        className="flex w-full max-w-sm flex-col gap-5 rounded-xl border border-white/8 bg-neutral-900 px-6 py-5 shadow-2xl"
      >
        <div className="flex items-center justify-between">
          <span id="settings-title" className="text-sm font-semibold text-neutral-100">
            Settings
          </span>
          <button
            onClick={onClose}
            aria-label="Close settings"
            className="rounded p-1 text-neutral-500 transition-colors hover:text-neutral-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-xs font-medium text-neutral-400" htmlFor="relay">
            Relay address
          </label>
          <div
            className={`flex items-center rounded-lg border bg-neutral-800/60 transition-colors focus-within:ring-1 ${
              isRelayEmpty
                ? 'border-red-500/40 focus-within:border-red-500/60 focus-within:ring-red-500/30'
                : 'border-white/8 focus-within:border-white/20 focus-within:ring-white/10'
            }`}
          >
            <span className="pl-3 text-sm text-neutral-500 select-none">moqt://</span>
            <input
              id="relay"
              type="text"
              value={relayDraft}
              placeholder="host:port"
              autoFocus
              spellCheck={false}
              onKeyDown={e => {
                if (e.key === 'Enter') save();
              }}
              onChange={e => setRelayDraft(e.target.value)}
              className="w-full bg-transparent py-2 pr-3 text-sm text-neutral-100 placeholder-neutral-600 outline-none"
            />
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-neutral-600">Default: {defaultRelay}</span>
            <button
              onClick={() => setRelayDraft(defaultRelay)}
              disabled={relayDraft.trim() === defaultRelay}
              className="text-[11px] text-neutral-400 transition-colors hover:text-neutral-200 disabled:invisible"
            >
              Reset
            </button>
          </div>
        </div>

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-lg bg-neutral-800 px-4 py-2 text-sm font-medium text-neutral-300 transition-colors hover:bg-neutral-700 active:bg-neutral-900"
          >
            Cancel
          </button>
          <button
            onClick={save}
            disabled={isRelayEmpty}
            className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-blue-500 active:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
