#!/usr/bin/env node
/**
 * Automated MOQtail SMPTE Bars Screenshot Test
 *
 * This script:
 * 1. Launches a browser
 * 2. Connects to the MOQtail relay
 * 3. Subscribes to the smpte/video track
 * 4. Captures 3 screenshots 10 seconds apart
 * 5. Saves them to disk
 */

import puppeteer from 'puppeteer';
import { writeFileSync, mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SCREENSHOTS_DIR = join(__dirname, 'screenshots');

// Ensure screenshots directory exists
try {
    mkdirSync(SCREENSHOTS_DIR, { recursive: true });
} catch (e) {
    // Directory already exists
}

console.log('🚀 Starting MOQtail automated test...\n');

async function runTest() {
    let browser;

    try {
        // Launch browser
        console.log('📱 Launching browser...');
        browser = await puppeteer.launch({
            headless: false, // Show browser for debugging
            args: [
                '--no-sandbox',
                '--disable-setuid-sandbox',
                '--disable-web-security',
                '--allow-insecure-localhost',
                '--enable-features=WebTransport',
                '--origin-to-force-quic-on=localhost:4433'
            ]
        });

        const page = await browser.newPage();

        // Set viewport
        await page.setViewport({ width: 1280, height: 800 });

        // Listen to console messages from the page
        page.on('console', msg => {
            const type = msg.type();
            const text = msg.text();
            if (type === 'error') {
                console.log(`  ❌ Browser Error: ${text}`);
            } else if (text.includes('success') || text.includes('Success') || text.includes('✅')) {
                console.log(`  ✅ ${text}`);
            } else if (text.includes('error') || text.includes('Error')) {
                console.log(`  ⚠️  ${text}`);
            }
        });

        // Navigate to the demo page
        console.log('🌐 Navigating to demo page...');
        await page.goto('http://localhost:8080/smpte-viewer.html', {
            waitUntil: 'networkidle2'
        });

        console.log('✅ Page loaded\n');

        // Wait for page to be ready
        await page.waitForSelector('#connectBtn', { timeout: 5000 });

        // Click Connect & Subscribe
        console.log('🔌 Clicking "Connect & Subscribe"...');
        await page.click('#connectBtn');

        // Wait for connection to establish
        await page.waitForFunction(
            () => {
                const status = document.getElementById('statusValue');
                return status && status.textContent === 'Receiving';
            },
            { timeout: 10000 }
        );

        console.log('✅ Connected and receiving video!\n');

        // Wait for frames to start decoding
        console.log('⏳ Waiting for video decoder to initialize...');
        await page.waitForFunction(
            () => {
                const frames = document.getElementById('framesValue');
                return frames && parseInt(frames.textContent) > 0;
            },
            { timeout: 15000 }
        );

        const frameCount = await page.$eval('#framesValue', el => el.textContent);
        console.log(`✅ Video decoding! Frames: ${frameCount}\n`);

        // Click Auto-Capture
        console.log('📸 Starting auto-capture (3 screenshots, 10s apart)...');
        await page.click('#autoBtn');

        // Wait for screenshots to complete (20+ seconds)
        console.log('⏳ Waiting 25 seconds for screenshots...\n');

        // Monitor progress
        for (let i = 0; i < 5; i++) {
            await new Promise(resolve => setTimeout(resolve, 5000));
            const screenshotCount = await page.$$eval('.screenshot-item', items => items.length);
            const currentFrames = await page.$eval('#framesValue', el => el.textContent);
            console.log(`   Progress: ${screenshotCount}/3 screenshots captured, ${currentFrames} frames decoded`);
        }

        // Verify screenshots were captured
        const screenshots = await page.$$('.screenshot-item');
        console.log(`\n✅ Captured ${screenshots.length} screenshots!\n`);

        if (screenshots.length < 3) {
            throw new Error(`Only ${screenshots.length} screenshots captured, expected 3`);
        }

        // Save each screenshot to disk
        console.log('💾 Saving screenshots to disk...');
        for (let i = 0; i < screenshots.length; i++) {
            const img = await screenshots[i].$('img');
            const src = await img.evaluate(el => el.src);

            // Extract base64 data
            const base64Data = src.replace(/^data:image\/png;base64,/, '');
            const buffer = Buffer.from(base64Data, 'base64');

            const filename = `smpte-screenshot-${i + 1}.png`;
            const filepath = join(SCREENSHOTS_DIR, filename);

            writeFileSync(filepath, buffer);
            console.log(`   ✅ Saved: ${filename}`);
        }

        // Get final stats
        const stats = await page.evaluate(() => {
            return {
                frames: document.getElementById('framesValue').textContent,
                keyframes: document.getElementById('keyframesValue').textContent,
                bitrate: document.getElementById('bitrateValue').textContent,
                screenshots: document.querySelectorAll('.screenshot-item').length
            };
        });

        console.log('\n📊 Final Statistics:');
        console.log(`   Frames Decoded: ${stats.frames}`);
        console.log(`   Keyframes: ${stats.keyframes}`);
        console.log(`   Bitrate: ${stats.bitrate}`);
        console.log(`   Screenshots: ${stats.screenshots}`);

        console.log('\n🎉 TEST PASSED! ✅');
        console.log(`\n📁 Screenshots saved to: ${SCREENSHOTS_DIR}`);
        console.log('   - smpte-screenshot-1.png');
        console.log('   - smpte-screenshot-2.png');
        console.log('   - smpte-screenshot-3.png\n');

        // Wait a moment before closing
        await new Promise(resolve => setTimeout(resolve, 2000));

        return true;

    } catch (error) {
        console.error('\n❌ TEST FAILED!');
        console.error(`Error: ${error.message}\n`);

        if (browser) {
            const page = (await browser.pages())[0];
            if (page) {
                // Take error screenshot
                const errorPath = join(SCREENSHOTS_DIR, 'error-screenshot.png');
                await page.screenshot({ path: errorPath });
                console.log(`Error screenshot saved to: ${errorPath}\n`);
            }
        }

        return false;

    } finally {
        if (browser) {
            await browser.close();
        }
    }
}

// Run the test
const success = await runTest();
process.exit(success ? 0 : 1);
