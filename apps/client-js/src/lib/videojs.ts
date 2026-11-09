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

/**
 * The Video.js v10 integration: `<moqtail-player>` and `<moqtail-skin>` around `<moqtail-video>`.
 * The player is the video preset plus the live feature, and the skin is the default video skin
 * with a Live button, which shows only for live streams.
 */

import '@videojs/html/video/skin';
import '@videojs/html/ui/live-button';
import { createPlayer, liveFeature, MediaAttachMixin } from '@videojs/html';
import { videoFeatures, VideoSkinElement } from '@videojs/html/video';
import { MOQtailVideoElement } from 'moqtail/player/component';

// The live skin's button styles, which the video skin doesn't carry
const LIVE_BUTTON_CSS = `
.media-live-button:not([data-live]) { display: none; }
.media-live-button {
  display: inline-flex; align-items: center; gap: calc(var(--media-spacing) * 1.5); width: auto;
  padding: calc(var(--media-spacing) * 2) calc(var(--media-spacing) * 3);
  font-size: calc(var(--media-spacing) * 2.75); font-weight: 600; line-height: 1;
  letter-spacing: .05em; text-transform: uppercase;
}
.media-live-button::before {
  content: ''; flex-shrink: 0; width: calc(var(--media-spacing) * 2); height: calc(var(--media-spacing) * 2);
  border-radius: 9999px; background-color: color-mix(in oklab, currentcolor 40%, transparent);
  transition: background-color .15s cubic-bezier(0, 0, .2, 1);
}
.media-live-button[data-live-edge]::before { background-color: oklch(65% .22 27); }
`;

const liveButtonSheet = new CSSStyleSheet();
liveButtonSheet.replaceSync(LIVE_BUTTON_CSS);

const { PlayerElement } = createPlayer({ features: [...videoFeatures, liveFeature] });

class MOQtailSkinElement extends VideoSkinElement {
  static {
    const template = VideoSkinElement.template!.cloneNode(true) as HTMLTemplateElement;
    const button = document.createElement('media-live-button');
    button.className = 'media-button media-live-button';
    template.content.querySelector('.video-time-slider-group')!.before(button);
    this.template = template;
  }

  constructor() {
    super();
    this.shadowRoot!.adoptedStyleSheets = [...this.shadowRoot!.adoptedStyleSheets, liveButtonSheet];
  }
}

// Video.js finds its media through this mixin, as its own media elements do
const elements: [string, CustomElementConstructor][] = [
  ['moqtail-video', class extends MediaAttachMixin(MOQtailVideoElement) {}],
  ['moqtail-player', class extends PlayerElement {}],
  ['moqtail-skin', MOQtailSkinElement],
];
for (const [name, element] of elements) {
  if (!customElements.get(name)) customElements.define(name, element);
}
