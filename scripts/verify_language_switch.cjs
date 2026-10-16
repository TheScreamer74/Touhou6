// Usage: node scripts/verify_language_switch.cjs <local-game-url> <output-dir>
// Requires Playwright (ticket #3 uses 1.48.2 / Firefox 131, build1465).
// Fresh synthetic options, no real saves; WebAudio output is disconnected.
// Serve your legally supplied game archives locally; no assets are included.
const {firefox}=require('playwright');
const fs=require('fs'); const path=require('path'); const assert=require('assert/strict');
(async()=>{
 const url=new URL(process.argv[2]||'http://127.0.0.1:8080/');assert(['localhost','127.0.0.1','[::1]'].includes(url.hostname),'Serve a local test fixture');
 const out=path.resolve(process.argv[3]||'logs/language-switch');fs.mkdirSync(out,{recursive:true});
 const browser=await firefox.launch({headless:true,...(process.env.TH06_FIREFOX_EXECUTABLE?{executablePath:process.env.TH06_FIREFOX_EXECUTABLE}:{})});
 const ctx=await browser.newContext({viewport:{width:1280,height:960}}); const page=await ctx.newPage(); const errors=[];page.on('pageerror',e=>errors.push(String(e)));
 // Same independent requestAnimationFrame observer as ticket #3. Reset before
 // each key press; retain all 12 half-second windows, including the first stall.
 await page.addInitScript(()=>{
  const frames=[]; let last=0; const errors=[];
  addEventListener('error',e=>errors.push(e.message));
  addEventListener('unhandledrejection',e=>errors.push(String(e.reason)));
  const tick=t=>{if(last)frames.push(t-last);last=t;requestAnimationFrame(tick);};requestAnimationFrame(tick);
  window.__bench={snapshot(){const f=frames.splice(0).sort((a,b)=>a-b);return{frames:{n:f.length,max:f.at(-1)||0},errors:errors.splice(0)};}};
 });
 await page.addInitScript(()=>{localStorage.setItem('th06:th06-options.bin','0302030000000001000101');
  // Silence the test at its source as well as the no-music fixture. Never
  // connect any WebAudio node to an output device during automation.
  const connect=AudioNode.prototype.connect;AudioNode.prototype.connect=function(destination,...args){if(destination instanceof AudioDestinationNode)return destination;return connect.call(this,destination,...args);};
 });
 await page.goto(process.argv[2]||'http://127.0.0.1:8080/',{waitUntil:'load'});await page.waitForSelector('body.playing',{timeout:300000});
 await page.waitForTimeout(5000);const press=async(k,n=1)=>{for(let i=0;i<n;i++){await page.keyboard.down(k);await page.waitForTimeout(50);await page.keyboard.up(k);await page.waitForTimeout(150);}};
 await page.locator('canvas').first().click({position:{x:5,y:5}});await press('z');await page.waitForTimeout(700);await press('ArrowUp');await page.waitForTimeout(600);await press('z');await page.waitForTimeout(1000);await press('ArrowDown',8);await page.waitForTimeout(300);
 await page.screenshot({path:out+'/options-before.png'});
 const steps=[];
 for(let i=0;i<6;i++){
  const before=await page.evaluate(()=>{window.__bench.snapshot();return window.__th06GameLanguage;});await press('ArrowRight');
  const trace=[];for(let s=0;s<12;s++){await page.waitForTimeout(500);trace.push(await page.evaluate(()=>window.__bench.snapshot()));}
  const after=await page.evaluate(()=>window.__th06GameLanguage);assert.notEqual(after,before,'The keyboard path must really change language');
  const worst=Math.max(...trace.map(t=>t.frames.max||0));steps.push({before,after,worstFrameMs:worst,trace});
  await page.screenshot({path:out+'/language-'+after+'-'+i+'.png'});console.log(JSON.stringify({before,after,worstFrameMs:worst}));
 }
 await press('x');await press('z');await page.waitForTimeout(1500);await page.screenshot({path:out+'/back-title.png'});
 const report={browser:browser.version(),viewport:{width:1280,height:960},sourceUrl:process.argv[2]||'http://127.0.0.1:8080/',silent:true,realSavesUsed:false,steps,errors};fs.writeFileSync(out+'/result.json',JSON.stringify(report,null,2));
 await browser.close();assert.equal(errors.length+steps.flatMap(s=>s.trace.flatMap(t=>t.errors)).length,0);if(!process.env.TH06_BASELINE)assert(steps.every(s=>s.worstFrameMs<=100),'language switch exceeds 100 ms');
})().catch(e=>{console.error(e);process.exit(1)});
