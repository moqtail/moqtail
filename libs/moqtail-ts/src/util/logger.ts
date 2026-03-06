export enum LogLevel {
  DEBUG = 0,
  LOG = 1,
  WARN = 2,
  ERROR = 3,
  NONE = 4,
}

const _config = {
  level: LogLevel.WARN,
  enabledModules: null as string[] | null,
}

export function setLogLevel(level: LogLevel): void {
  _config.level = level
}

export function setLogEnabledModules(modules: string[] | null): void {
  _config.enabledModules = modules
}

export class Logger {
  constructor(private readonly module: string) {}

  private isEnabled(level: LogLevel): boolean {
    if (level < _config.level) return false
    if (_config.enabledModules !== null && !_config.enabledModules.includes(this.module)) return false
    return true
  }

  debug(...args: unknown[]): void {
    if (this.isEnabled(LogLevel.DEBUG)) console.debug(`[MOQtail][${this.module}]`, ...args)
  }

  log(...args: unknown[]): void {
    if (this.isEnabled(LogLevel.LOG)) console.log(`[MOQtail][${this.module}]`, ...args)
  }

  warn(...args: unknown[]): void {
    if (this.isEnabled(LogLevel.WARN)) console.warn(`[MOQtail][${this.module}]`, ...args)
  }

  error(...args: unknown[]): void {
    if (this.isEnabled(LogLevel.ERROR)) console.error(`[MOQtail][${this.module}]`, ...args)
  }
}

export function createLogger(module: string): Logger {
  return new Logger(module)
}
