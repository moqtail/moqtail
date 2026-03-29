import { cn } from '@/lib/utils';

interface ChartLine {
  label: string;
  color: string;
  data: number[];
}

interface ThresholdLine {
  value: number;
  color: string;
  label: string;
}

interface MetricsChartProps {
  title: string;
  lines: ChartLine[];
  thresholds?: ThresholdLine[];
  yUnit: string;
  width?: number;
  height?: number;
  step?: boolean;
}

export function MetricsChart({
  title,
  lines,
  thresholds = [],
  yUnit,
  width = 300,
  height = 80,
  step = false,
}: MetricsChartProps) {
  let yMin = Infinity;
  let yMax = -Infinity;
  for (const line of lines) {
    for (const v of line.data) {
      if (v < yMin) yMin = v;
      if (v > yMax) yMax = v;
    }
  }
  for (const t of thresholds) {
    if (t.value < yMin) yMin = t.value;
    if (t.value > yMax) yMax = t.value;
  }
  if (!isFinite(yMin)) yMin = 0;
  if (!isFinite(yMax)) yMax = 1;
  if (yMax === yMin) yMax = yMin + 1;

  const pad = 4;
  const plotW = width - pad * 2;
  const plotH = height - pad * 2;

  const toX = (i: number, len: number) => pad + (len > 1 ? (i / (len - 1)) * plotW : plotW / 2);
  const toY = (v: number) => pad + plotH - ((v - yMin) / (yMax - yMin)) * plotH;

  const buildPath = (data: number[]): string => {
    if (data.length === 0) return '';
    const points = data.map((v, i) => ({ x: toX(i, data.length), y: toY(v) }));
    if (step) {
      let d = `M${points[0].x},${points[0].y}`;
      for (let i = 1; i < points.length; i++) {
        d += ` H${points[i].x} V${points[i].y}`;
      }
      return d;
    }
    return points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x},${p.y}`).join(' ');
  };

  return (
    <div className={cn()}>
      <p className="mb-1 text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
        {title}
      </p>
      <svg
        width={width}
        height={height}
        className="w-full overflow-visible"
        aria-label={`${title} (${yUnit})`}
      >
        {thresholds.map((t, i) => {
          const y = toY(t.value);
          return (
            <g key={i}>
              <line
                x1={pad}
                y1={y}
                x2={width - pad}
                y2={y}
                stroke={t.color}
                strokeWidth="0.5"
                strokeDasharray="4,3"
              />
              <text x={width - pad} y={y - 2} textAnchor="end" fill={t.color} fontSize="8">
                {t.label}
              </text>
            </g>
          );
        })}
        {lines.map((line, i) => (
          <path
            key={i}
            d={buildPath(line.data)}
            fill="none"
            stroke={line.color}
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
        ))}
      </svg>
      {lines.length > 1 && (
        <div className="mt-0.5 flex gap-3">
          {lines.map((line, i) => (
            <span key={i} className="flex items-center gap-1 text-[9px] text-neutral-500">
              <span
                className="inline-block h-1.5 w-3 rounded"
                style={{ backgroundColor: line.color }}
              />
              {line.label}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
