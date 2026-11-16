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
 * with a Live button, which shows only for live streams, and no time slider without anything to seek.
 */

import '@videojs/html/video/skin';
import '@videojs/html/ui/live-button';
import { createPlayer, liveFeature, MediaAttachMixin } from '@videojs/html';
import { videoFeatures, VideoSkinElement } from '@videojs/html/video';
import { MOQtailVideoElement } from 'moqtail/player/component';
import SKIN_CSS from './videojs.css?raw';

const skinSheet = new CSSStyleSheet();
skinSheet.replaceSync(SKIN_CSS);

const { PlayerElement } = createPlayer({ features: [...videoFeatures, liveFeature] });

class MOQtailSkinElement extends VideoSkinElement {
  static {
    const template = VideoSkinElement.template!.cloneNode(true) as HTMLTemplateElement;
    const button = document.createElement('media-live-button');
    button.className = 'media-button media-live-button';
    const spacer = document.createElement('div');
    spacer.className = 'video-controls-spacer';
    spacer.setAttribute('aria-hidden', 'true');
    const slider = template.content.querySelector('.video-time-slider-group')!;
    slider.before(button);
    slider.after(spacer);
    this.template = template;
  }

  constructor() {
    super();
    this.shadowRoot!.adoptedStyleSheets = [...this.shadowRoot!.adoptedStyleSheets, skinSheet];
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
