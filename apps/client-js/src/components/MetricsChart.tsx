export interface ChartLine {
  label: string;
  color: string;
  data: number[];
  unit: string;
  unitLabel: string;
  step?: boolean;
}

interface MetricsChartProps {
  lines: ChartLine[];
  width?: number;
  height?: number;
}

const TICK_COUNT = 5;

function fmtTick(v: number, unit: string): string {
  if (unit === 'kbps') {
    if (Math.abs(v) >= 1000) return `${(v / 1000).toFixed(1)}M`;
    return `${v.toFixed(0)}k`;
  }
  if (unit === 's') return v.toFixed(1);
  if (unit === 'ms') return v.toFixed(0);
  if (unit === 'x') return v.toFixed(2);
  if (unit === 'count') return v.toFixed(0);
  return v.toFixed(1);
}

type AxisSide = 'left' | 'right';

export function MetricsChart({ lines, width = 300, height = 140 }: MetricsChartProps) {
  if (lines.length === 0) return null;

  // ── Assign units to axes ──
  // 1st unique unit → left, 2nd → right, 3rd+ → fall back to left
  const unitOrder: string[] = [];
  for (const line of lines) {
    if (!unitOrder.includes(line.unit)) unitOrder.push(line.unit);
  }

  const unitToAxis: Record<string, AxisSide> = {};
  for (let i = 0; i < unitOrder.length; i++) {
    unitToAxis[unitOrder[i]] = i === 1 ? 'right' : 'left';
  }

  const hasRightAxis = unitOrder.length >= 2;

  // ── Build axis labels by concatenating unique unitLabels with " / " ──
  const axisLabels: Record<AxisSide, string[]> = { left: [], right: [] };
  for (const line of lines) {
    const side = unitToAxis[line.unit];
    if (!axisLabels[side].includes(line.unitLabel)) {
      axisLabels[side].push(line.unitLabel);
    }
  }
  const leftLabel = axisLabels.left.join(' / ');
  const rightLabel = axisLabels.right.join(' / ');

  // ── Compute Y range per axis (all lines on same axis share one scale) ──
  const axisRange: Record<AxisSide, { min: number; max: number }> = {
    left: { min: Infinity, max: -Infinity },
    right: { min: Infinity, max: -Infinity },
  };
  for (const line of lines) {
    const side = unitToAxis[line.unit];
    for (const v of line.data) {
      if (v < axisRange[side].min) axisRange[side].min = v;
      if (v > axisRange[side].max) axisRange[side].max = v;
    }
  }

  // Ensure valid ranges, begin at zero like DASH.js
  for (const side of ['left', 'right'] as AxisSide[]) {
    const r = axisRange[side];
    if (!isFinite(r.min)) r.min = 0;
    if (!isFinite(r.max)) r.max = 1;
    if (r.min > 0) r.min = 0; // beginAtZero
    if (r.max === r.min) r.max = r.min + 1;
    // 5% top padding
    r.max = r.max + (r.max - r.min) * 0.05;
  }

  // ── Layout ──
  const padL = 40;
  const padR = hasRightAxis ? 40 : 8;
  const padT = 6;
  const padB = 6;
  const plotW = width - padL - padR;
  const plotH = height - padT - padB;

  const toX = (i: number, len: number) => padL + (len > 1 ? (i / (len - 1)) * plotW : plotW / 2);

  const toY = (v: number, unit: string) => {
    const side = unitToAxis[unit] ?? 'left';
    const r = axisRange[side];
    return padT + plotH - ((v - r.min) / (r.max - r.min)) * plotH;
  };

  // ── Path builder ──
  const buildPath = (line: ChartLine): string => {
    if (line.data.length === 0) return '';
    const points = line.data.map((v, i) => ({
      x: toX(i, line.data.length),
      y: toY(v, line.unit),
    }));
    if (line.step) {
      let d = `M${points[0].x},${points[0].y}`;
      for (let i = 1; i < points.length; i++) {
        d += ` H${points[i].x} V${points[i].y}`;
      }
      return d;
    }
    return points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x},${p.y}`).join(' ');
  };

  // ── Tick values for an axis ──
  const ticks = (side: AxisSide): number[] => {
    const r = axisRange[side];
    const result: number[] = [];
    for (let i = 0; i < TICK_COUNT; i++) {
      result.push(r.min + (i / (TICK_COUNT - 1)) * (r.max - r.min));
    }
    return result;
  };

  // ── Representative unit for tick formatting (first unit assigned to that axis) ──
  const leftTickUnit = unitOrder[0];
  const rightTickUnit = unitOrder[1] ?? unitOrder[0];

  return (
    <div>
      <p className="mb-1 text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
        Metrics Chart
      </p>
      <svg
        width={width}
        height={height}
        className="w-full overflow-visible"
        aria-label="Metrics chart"
      >
        {/* Grid lines from left axis */}
        {ticks('left').map((v, i) => {
          const y = toY(v, leftTickUnit);
          return (
            <line
              key={`grid-${i}`}
              x1={padL}
              y1={y}
              x2={padL + plotW}
              y2={y}
              stroke="#333"
              strokeWidth="0.5"
              strokeDasharray="2,2"
            />
          );
        })}

        {/* Left Y-axis ticks */}
        {ticks('left').map((v, i) => (
          <text
            key={`lt-${i}`}
            x={padL - 4}
            y={toY(v, leftTickUnit) + 3}
            textAnchor="end"
            fill="#737373"
            fontSize="7"
          >
            {fmtTick(v, leftTickUnit)}
          </text>
        ))}
        {/* Left axis label */}
        <text
          x={6}
          y={padT + plotH / 2}
          textAnchor="middle"
          fill="#737373"
          fontSize="7"
          transform={`rotate(-90, 6, ${padT + plotH / 2})`}
        >
          {leftLabel}
        </text>

        {/* Right Y-axis ticks + label */}
        {hasRightAxis &&
          ticks('right').map((v, i) => (
            <text
              key={`rt-${i}`}
              x={padL + plotW + 4}
              y={toY(v, rightTickUnit) + 3}
              textAnchor="start"
              fill="#737373"
              fontSize="7"
            >
              {fmtTick(v, rightTickUnit)}
            </text>
          ))}
        {hasRightAxis && (
          <text
            x={width - 6}
            y={padT + plotH / 2}
            textAnchor="middle"
            fill="#737373"
            fontSize="7"
            transform={`rotate(90, ${width - 6}, ${padT + plotH / 2})`}
          >
            {rightLabel}
          </text>
        )}

        {/* Plot area border */}
        <rect
          x={padL}
          y={padT}
          width={plotW}
          height={plotH}
          fill="none"
          stroke="#404040"
          strokeWidth="0.5"
        />

        {/* Data lines */}
        {lines.map((line, i) => (
          <path
            key={i}
            d={buildPath(line)}
            fill="none"
            stroke={line.color}
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
        ))}
      </svg>

      {/* Legend */}
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5">
        {lines.map((line, i) => (
          <span key={i} className="flex items-center gap-1 text-[9px] text-neutral-400">
            <span
              className="inline-block h-2 w-2 rounded-sm"
              style={{ backgroundColor: line.color }}
            />
            {line.label}
          </span>
        ))}
      </div>
    </div>
  );
}
