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

import { cn } from '@/lib/utils';
import { type AbrSettings } from '@/lib/abr/types';

export interface SettingsPanelProps {
  open: boolean;
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
      <span className="text-xs text-neutral-400">{label}</span>
      <input
        type="number"
        value={value === -1 ? '' : value}
        placeholder={placeholder}
        onInput={e => {
          const v = (e.target as HTMLInputElement).value;
          onChange(v === '' ? -1 : parseFloat(v));
        }}
        className="w-20 rounded border border-neutral-700 bg-neutral-900 px-2 py-0.5 text-right font-mono text-xs text-neutral-200 focus:border-blue-500 focus:outline-none"
      />
    </div>
  );
}

function OptionCard({ title, children }: { title: string; children: preact.ComponentChildren }) {
  return (
    <div
      className="flex-none snap-start overflow-hidden rounded-md border border-neutral-700/60 bg-neutral-900/80"
      style={{ width: '220px' }}
    >
      <div className="border-b border-neutral-700/60 bg-neutral-800/60 px-3 py-1.5">
        <span className="text-[11px] font-semibold tracking-widest text-blue-400 uppercase">
          {title}
        </span>
      </div>
      <div className="scrollbar-thin max-h-[320px] overflow-y-auto px-3 py-2">{children}</div>
    </div>
  );
}

function SectionLabel({ children }: { children: preact.ComponentChildren }) {
  return (
    <p className="mt-2 mb-1 border-b border-neutral-700/50 pb-0.5 text-[10px] font-semibold tracking-widest text-blue-400/80 uppercase first:mt-0">
      {children}
    </p>
  );
}

const ABR_RULES = [
  'ThroughputRule',
  'BolaRule',
  'InsufficientBufferRule',
  'SwitchHistoryRule',
  'DroppedFramesRule',
  'AbandonRequestsRule',
] as const;
const LOW_LATENCY_RULES = ['L2ARule', 'LoLPRule'] as const;

export function SettingsPanel({ open, settings, onSettingsChange }: SettingsPanelProps) {
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
    <div
      className={cn(
        'overflow-hidden border-b border-white/6 bg-neutral-950/80 transition-all duration-300',
        open ? 'max-h-[420px] px-4 py-3 opacity-100' : 'max-h-0 px-4 py-0 opacity-0',
      )}
    >
      {/* Horizontal scroll wrapper */}
      <div className="relative">
        <div className="scrollbar-thin scrollbar-track-transparent scrollbar-thumb-neutral-700 flex snap-x snap-proximity gap-3 overflow-x-auto pb-2">
          {/* ABR Card */}
          <OptionCard title="ABR">
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
          </OptionCard>

          {/* ABR Rules Card */}
          <OptionCard title="ABR Rules">
            {ABR_RULES.map(ruleName => (
              <SettingCheckbox
                key={ruleName}
                label={ruleName}
                checked={settings.rules[ruleName]?.active ?? false}
                onChange={v => updateRule(ruleName, v)}
              />
            ))}
            <SectionLabel>Low Latency</SectionLabel>
            {LOW_LATENCY_RULES.map(ruleName => (
              <SettingCheckbox
                key={ruleName}
                label={ruleName}
                checked={settings.rules[ruleName]?.active ?? false}
                onChange={v => updateRule(ruleName, v)}
              />
            ))}
          </OptionCard>

          {/* Buffer Card */}
          <OptionCard title="Buffer">
            <NumberInput
              label="Buffer Time (s)"
              value={settings.bufferTimeDefault}
              placeholder="18"
              onChange={v => onSettingsChange({ ...settings, bufferTimeDefault: v })}
            />
            <NumberInput
              label="Stable Buffer (s)"
              value={settings.stableBufferTime}
              placeholder="18"
              onChange={v => onSettingsChange({ ...settings, stableBufferTime: v })}
            />
            <NumberInput
              label="BW Safety Factor"
              value={settings.bandwidthSafetyFactor}
              placeholder="0.9"
              onChange={v => onSettingsChange({ ...settings, bandwidthSafetyFactor: v })}
            />
          </OptionCard>

          {/* Initial Settings Card */}
          <OptionCard title="Initial Settings">
            <NumberInput
              label="Initial Bitrate (kbps)"
              value={settings.initialBitrate}
              placeholder="auto"
              onChange={v => onSettingsChange({ ...settings, initialBitrate: v })}
            />
            <NumberInput
              label="Min Bitrate (kbps)"
              value={settings.minBitrate}
              placeholder="none"
              onChange={v => onSettingsChange({ ...settings, minBitrate: v })}
            />
            <NumberInput
              label="Max Bitrate (kbps)"
              value={settings.maxBitrate}
              placeholder="none"
              onChange={v => onSettingsChange({ ...settings, maxBitrate: v })}
            />
          </OptionCard>

          {/* EWMA Card */}
          <OptionCard title="EWMA">
            <NumberInput
              label="Fast Half-life (s)"
              value={settings.ewma.throughputFastHalfLifeSeconds}
              placeholder="3"
              onChange={v =>
                onSettingsChange({
                  ...settings,
                  ewma: { ...settings.ewma, throughputFastHalfLifeSeconds: v },
                })
              }
            />
            <NumberInput
              label="Slow Half-life (s)"
              value={settings.ewma.throughputSlowHalfLifeSeconds}
              placeholder="8"
              onChange={v =>
                onSettingsChange({
                  ...settings,
                  ewma: { ...settings.ewma, throughputSlowHalfLifeSeconds: v },
                })
              }
            />
          </OptionCard>
        </div>

        {/* Right-edge fade indicator */}
        <div className="pointer-events-none absolute top-0 right-0 bottom-2 w-10 bg-gradient-to-r from-transparent to-neutral-950/80" />
      </div>
    </div>
  );
}
