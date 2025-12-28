#!/usr/bin/env node
/**
 * Simple Automated Screenshot Test
 */

import puppeteer from 'puppeteer';
import { writeFileSync, mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SCREENSHOTS_DIR = join(__dirname, 'screenshots');

mkdirSync(SCREENSHOTS_DIR, { recursive: true });

console.log('🚀 Starting simple screenshot test...\n');

async function runTest() {
    const browser = await puppeteer.launch({
        headless: false,
        args: ['--no-sandbox', '--disable-setuid-sandbox']
    });

    try {
        const page = await browser.newPage();
        await page.setViewport({ width: 1280, height: 800 });

        page.on('console', msg => {
            const text = msg.text();
            if (text.includes('✅') || text.includes('Screenshot')) {
                console.log(`  ${text}`);
            }
        });

        console.log('📱 Opening test page...');
        await page.goto('http://localhost:8080/simple-test.html');
        console.log('✅ Page loaded\n');

        console.log('📸 Running auto-capture function...');
        await page.evaluate(() => autoCapture());

        // Wait for screenshots (3 screenshots, 10s apart = ~20s total)
        console.log('⏳ Waiting for screenshots to complete (25 seconds)...\n');
        await new Promise(r => setTimeout(r, 25000));

        // Save screenshots
        const screenshots = await page.$$eval('#screenshots img', imgs =>
            imgs.map(img => img.src)
        );

        console.log(`\n💾 Saving ${screenshots.length} screenshots...\n`);
        screenshots.forEach((src, i) => {
            const base64Data = src.replace(/^data:image\/png;base64,/, '');
            const buffer = Buffer.from(base64Data, 'base64');
            const filename = `smpte-screenshot-${i + 1}.png`;
            const filepath = join(SCREENSHOTS_DIR, filename);
            writeFileSync(filepath, buffer);
            console.log(`   ✅ Saved: ${filename}`);
        });

        console.log(`\n🎉 TEST PASSED! ${screenshots.length} screenshots captured`);
        console.log(`\n📁 Screenshots saved to: ${SCREENSHOTS_DIR}\n`);

        await new Promise(r => setTimeout(r, 2000));
        return true;

    } catch (error) {
        console.error(`\n❌ TEST FAILED: ${error.message}\n`);
        return false;
    } finally {
        await browser.close();
    }
}

const success = await runTest();
process.exit(success ? 0 : 1);
