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

import clsx, { type ClassValue } from 'clsx';
import { LogLevel, MOQtailClient } from 'moqtail';
import { twMerge } from 'tailwind-merge';
import { logger, Logger } from './logger';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

type AppLogLevel = 'debug' | 'info' | 'warn' | 'error';

export function parseLogLevel(
  raw: string | null | undefined,
): { app: AppLogLevel; moq: LogLevel } | null {
  switch (raw?.toLowerCase()) {
    case 'debug':
      return { app: 'debug', moq: LogLevel.DEBUG };
    case 'log':
    case 'info':
      return { app: 'info', moq: LogLevel.LOG };
    case 'warn':
      return { app: 'warn', moq: LogLevel.WARN };
    case 'error':
      return { app: 'error', moq: LogLevel.ERROR };
    case 'none':
      return { app: 'error', moq: LogLevel.NONE };
    default:
      return null;
  }
}

export function applyLogLevel(level: { app: AppLogLevel; moq: LogLevel }) {
  Logger.setLevel(level.app);
  MOQtailClient.setLogLevel(level.moq);
}

(window as any).setMOQtailLogLevel = (raw: string) => {
  const level = parseLogLevel(raw);
  if (!level) {
    logger.warn('player', `MOQtail unknown log level: "${raw}"`);
    return;
  }
  applyLogLevel(level);
  logger.info('player', `MOQtail log level set to "${raw}"`);
};
