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

// Incremental parsers for the API's two streaming formats. Both take text as it
// arrives, in pieces of any size, and hand out each complete record once.

const RECORD_SEPARATOR = '\u001e';

/** Splits JSON-SEQ (qlog trace files): each record starts with RS and ends with LF. */
export class JsonSeqParser {
  private buffer = '';

  /** Adds text and returns the records it completed, parsed. */
  push(text: string): unknown[] {
    this.buffer += text;
    const records: unknown[] = [];
    let end = this.buffer.indexOf('\n');
    while (end >= 0) {
      const record = this.buffer.slice(0, end).replace(RECORD_SEPARATOR, '').trim();
      this.buffer = this.buffer.slice(end + 1);
      if (record) {
        try {
          records.push(JSON.parse(record));
        } catch {
          // A record that does not parse is skipped; the next one starts fresh.
        }
      }
      end = this.buffer.indexOf('\n');
    }
    return records;
  }
}

export interface SseMessage {
  event: string;
  data: string;
}

/** Splits Server-Sent Events: fields on lines, messages separated by a blank line. */
export class SseParser {
  private buffer = '';

  push(text: string): SseMessage[] {
    this.buffer += text.replace(/\r\n/g, '\n');
    const messages: SseMessage[] = [];
    let end = this.buffer.indexOf('\n\n');
    while (end >= 0) {
      const block = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 2);
      let event = 'message';
      const data: string[] = [];
      for (const line of block.split('\n')) {
        if (line.startsWith('event:')) event = line.slice(6).trim();
        else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
      }
      // A block with only a comment, such as a keep-alive, carries no message.
      if (data.length > 0) messages.push({ event, data: data.join('\n') });
      end = this.buffer.indexOf('\n\n');
    }
    return messages;
  }
}
