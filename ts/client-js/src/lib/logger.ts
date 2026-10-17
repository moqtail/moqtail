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

type LogLevel = 'debug' | 'info' | 'warn' | 'error';

const LOG_LEVELS: Record<LogLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

const LEVEL_COLORS: Record<LogLevel, string> = {
  debug: '\x1b[36m', // Cyan
  info: '\x1b[32m', // Green
  warn: '\x1b[33m', // Yellow
  error: '\x1b[31m', // Red
};

const RESET = '\x1b[0m';

export class Logger {
  private static componentLevels: Map<string, LogLevel> = new Map();
  private defaultLevel: LogLevel = 'warn';

  static setLevel(levels: string | Record<string, LogLevel>): void {
    if (typeof levels === 'string') {
      Logger.componentLevels.clear();
      Logger.componentLevels.set('*', levels as LogLevel);
    } else {
      Object.entries(levels).forEach(([component, level]) => {
        Logger.componentLevels.set(component, level);
      });
    }
  }

  setDefaultLevel(level: LogLevel): void {
    this.defaultLevel = level;
  }

  private shouldLog(component: string, level: LogLevel): boolean {
    const componentLevel =
      Logger.componentLevels.get('*') || Logger.componentLevels.get(component) || this.defaultLevel;
    return LOG_LEVELS[level] >= LOG_LEVELS[componentLevel];
  }

  private log(component: string, level: LogLevel, message: string, ...args: any[]): void {
    if (!this.shouldLog(component, level)) return;

    const coloredLevel = `${LEVEL_COLORS[level]}${level.toUpperCase()}${RESET}`;
    const prefix = `[${coloredLevel}] [${component}]`;

    const consoleMethod =
      level === 'debug'
        ? console.debug
        : level === 'info'
          ? console.info
          : level === 'warn'
            ? console.warn
            : console.error;

    consoleMethod(prefix, message, ...args);
  }

  debug(component: string, message: string, ...args: any[]): void {
    this.log(component, 'debug', message, ...args);
  }

  info(component: string, message: string, ...args: any[]): void {
    this.log(component, 'info', message, ...args);
  }

  warn(component: string, message: string, ...args: any[]): void {
    this.log(component, 'warn', message, ...args);
  }

  error(component: string, message: string, ...args: any[]): void {
    this.log(component, 'error', message, ...args);
  }
}

export const logger = new Logger();
export type { LogLevel };
