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

import { useState } from 'preact/hooks';
import { cn } from '@/lib/utils';
import { type AbrSettings } from '@/lib/abr/types';

interface SettingsPanelProps {
  settings: AbrSettings;
  onSettingsChange: (settings: AbrSettings) => void;
}

function SettingCheckbox({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-2 py-0.5">
      <input
        type="checkbox"
        checked={checked}
        onChange={e => onChange((e.target as HTMLInputElement).checked)}
        className="accent-blue-500"
      />
      <span className="text-xs text-neutral-300">{label}</span>
    </label>
  );
}

function NumberInput({
  label,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  value: number;
  placeholder: string;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-2 py-0.5">
      <span className="text-xs text-neutral-500">{label}</span>
      <input
        type="number"
        value={value === -1 ? '' : value}
        placeholder={placeholder}
        onInput={e => {
          const v = (e.target as HTMLInputElement).value;
          onChange(v === '' ? -1 : parseFloat(v));
        }}
        className="w-24 rounded border border-neutral-700 bg-neutral-900 px-2 py-0.5 text-right font-mono text-xs text-neutral-200 focus:border-blue-500 focus:outline-none"
      />
    </div>
  );
}

const RULE_GROUPS = {
  'ABR Rules': ['ThroughputRule', 'BolaRule', 'InsufficientBufferRule', 'SwitchHistoryRule', 'DroppedFramesRule', 'AbandonRequestsRule'],
  'Low Latency': ['L2ARule', 'LoLPRule'],
} as const;

export function SettingsPanel({ settings, onSettingsChange }: SettingsPanelProps) {
  const [open, setOpen] = useState(false);

  const updateRule = (ruleName: string, active: boolean) => {
    const updated = { ...settings };
    updated.rules = { ...updated.rules };
    updated.rules[ruleName] = { ...updated.rules[ruleName], active };

    if (active && ruleName === 'L2ARule') {
      updated.rules.LoLPRule = { ...updated.rules.LoLPRule, active: false };
    } else if (active && ruleName === 'LoLPRule') {
      updated.rules.L2ARule = { ...updated.rules.L2ARule, active: false };
    }

    onSettingsChange(updated);
  };

  return (
    <div className={cn('border-t border-white/6')}>
      <button
        onClick={() => setOpen(o => !o)}
        className="flex w-full items-center justify-between px-3 py-2 text-[11px] font-semibold tracking-widest text-neutral-500 uppercase hover:text-neutral-300"
      >
        Options <span>{open ? '▴' : '▾'}</span>
      </button>

      {open && (
        <div className="space-y-3 px-3 pb-3">
          <div>
            <p className="mb-1 text-[11px] font-semibold tracking-widest text-neutral-400 uppercase">ABR</p>
            <SettingCheckbox
              label="Fast Switching"
              checked={settings.fastSwitching}
              onChange={v => onSettingsChange({ ...settings, fastSwitching: v })}
            />
            <SettingCheckbox
              label="Video Auto Switch"
              checked={settings.videoAutoSwitch}
              onChange={v => onSettingsChange({ ...settings, videoAutoSwitch: v })}
            />
          </div>

          {Object.entries(RULE_GROUPS).map(([groupName, ruleNames]) => (
            <div key={groupName}>
              <p className="mb-1 text-[10px] font-semibold tracking-widest text-neutral-500 uppercase">
                {groupName}
              </p>
              {ruleNames.map(ruleName => (
                <SettingCheckbox
                  key={ruleName}
                  label={ruleName}
                  checked={settings.rules[ruleName]?.active ?? false}
                  onChange={v => updateRule(ruleName, v)}
                />
              ))}
            </div>
          ))}

          <div>
            <p className="mb-1 text-[10px] font-semibold tracking-widest text-neutral-500 uppercase">
              Initial Settings (Video)
            </p>
            <NumberInput
              label="Initial Bitrate (kbps)"
              value={settings.initialBitrate}
              placeholder=""
              onChange={v => onSettingsChange({ ...settings, initialBitrate: v })}
            />
            <NumberInput
              label="Min Bitrate (kbps)"
              value={settings.minBitrate}
              placeholder=""
              onChange={v => onSettingsChange({ ...settings, minBitrate: v })}
            />
            <NumberInput
              label="Max Bitrate (kbps)"
              value={settings.maxBitrate}
              placeholder=""
              onChange={v => onSettingsChange({ ...settings, maxBitrate: v })}
            />
          </div>
        </div>
      )}
    </div>
  );
}
