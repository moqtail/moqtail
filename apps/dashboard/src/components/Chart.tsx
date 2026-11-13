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

import { useEffect, useRef } from 'preact/hooks';
import uPlot from 'uplot';
import 'uplot/dist/uPlot.min.css';

// Axis labels and grid lines in the dark theme's neutrals.
const AXIS = '#737373';
const GRID = 'rgba(255, 255, 255, 0.06)';

export interface Series {
  label: string;
  /** A CSS colour. */
  color: string;
}

const TIME_STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

const clockLabel = (seconds: number) =>
  new Date(seconds * 1000).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });

/**
 * A time-series line chart. `times` are seconds since the epoch; each entry of
 * `values` is one series, aligned with `times`. The numbers behind the chart are
 * shown in a table for screen readers.
 */
export function Chart(props: {
  title: string;
  times: number[];
  values: (number | null)[][];
  series: Series[];
  format: (value: number) => string;
  height?: number;
}) {
  const host = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);

  useEffect(() => {
    if (!host.current) return;
    const options: uPlot.Options = {
      width: host.current.clientWidth || 400,
      height: props.height ?? 160,
      legend: { show: true },
      axes: [
        {
          stroke: AXIS,
          grid: { stroke: GRID },
          ticks: { stroke: GRID },
          // Whole seconds and up, labelled with the time of day only: uPlot's own
          // labels step by half seconds on a short span and put a date under the
          // first tick, where the chart's edge cuts it off.
          incrs: TIME_STEPS,
          space: 70,
          values: (_u, ticks) => ticks.map(clockLabel),
        },
        {
          stroke: AXIS,
          grid: { stroke: GRID },
          ticks: { stroke: GRID },
          values: (_u, ticks) => ticks.map(t => props.format(t)),
          size: 70,
        },
      ],
      series: [
        {},
        ...props.series.map(s => ({
          label: s.label,
          stroke: s.color,
          width: 1.5,
          // The legend shows values the same way as the axis, not raw floats.
          value: (_u: uPlot, v: number | null) => (v === null ? '–' : props.format(v)),
        })),
      ],
    };
    plot.current = new uPlot(
      options,
      [props.times, ...props.values] as uPlot.AlignedData,
      host.current,
    );
    const resize = new ResizeObserver(() => {
      if (host.current && plot.current) {
        plot.current.setSize({ width: host.current.clientWidth, height: props.height ?? 160 });
      }
    });
    resize.observe(host.current);
    return () => {
      resize.disconnect();
      plot.current?.destroy();
      plot.current = null;
    };
  }, [props.series.map(s => s.label).join()]);

  useEffect(() => {
    plot.current?.setData([props.times, ...props.values] as uPlot.AlignedData);
  }, [props.times, props.values]);

  const last = props.times.length - 1;
  return (
    <figure>
      <div ref={host} role="img" aria-label={props.title} />
      <figcaption class="sr-only">
        {props.title}:{' '}
        {last >= 0
          ? props.series
              .map(
                (s, i) =>
                  `${s.label} ${props.values[i][last] === null ? 'no data' : props.format(props.values[i][last]!)}`,
              )
              .join(', ')
          : 'no data yet'}
      </figcaption>
    </figure>
  );
}
