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
  private isEnabled(module: string, level: LogLevel): boolean {
    if (level < _config.level) return false
    if (_config.enabledModules !== null && !_config.enabledModules.includes(module)) return false
    return true
  }

  debug(module: string, ...args: unknown[]): void {
    if (this.isEnabled(module, LogLevel.DEBUG)) console.debug(`[MOQtail][${module}]`, ...args)
  }

  log(module: string, ...args: unknown[]): void {
    if (this.isEnabled(module, LogLevel.LOG)) console.log(`[MOQtail][${module}]`, ...args)
  }

  warn(module: string, ...args: unknown[]): void {
    if (this.isEnabled(module, LogLevel.WARN)) console.warn(`[MOQtail][${module}]`, ...args)
  }

  error(module: string, ...args: unknown[]): void {
    if (this.isEnabled(module, LogLevel.ERROR)) console.error(`[MOQtail][${module}]`, ...args)
  }
}

export const logger = new Logger()
