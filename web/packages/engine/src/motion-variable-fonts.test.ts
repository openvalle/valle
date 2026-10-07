import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine, compile_motion_jsx } from "../generated/web/valle_engine.js";
import { compileMotionJsxWithWasm } from "./compiler.ts";
import { decodePackedAbi, RESOURCE_REQUESTS_ABI, type PackedValue } from "./abi/packed.ts";
import { CanvasKitExecutor, type CanvasKitExternalObject } from "./executor/canvaskit/executor.ts";

const root = resolve(import.meta.dir, "../../../../");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const { default: CanvasKitInit } = await import("canvaskit-wasm") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
type Ink = { width: number; count: number; coverage: number };
function ink(pixels: Uint8Array): Ink {
  let left=640, right=-1, count=0, coverage=0;
  for (let i=0;i<pixels.length;i+=4) {
    const value=pixels[i]!;
    coverage+=Math.max(0,value-16);
    if (value>128) { const x=(i/4)%640; left=Math.min(left,x); right=Math.max(right,x); count++; }
  }
  return { width:right-left+1,count,coverage };
}

test("variable Noto axes produce continuous ink and identical random-access frames on both backends", async () => {
  const ck=await CanvasKitInit({locateFile:()=>Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm",import.meta.dir)});
  initSync({module:await readFile(new URL("../generated/web/valle_engine_bg.wasm",import.meta.url))});
  const font=new Uint8Array(await readFile(join(root,"assets/fonts/noto/NotoSans-Variable.ttf")));
  const cjk=new Uint8Array(await readFile(join(root,"assets/fonts/noto/NotoSansCJKsc-Variable.otf")));
  const fontBytes=new Map([font,cjk].map(bytes=>[`sha256:${new Bun.CryptoHasher("sha256").update(bytes).digest("hex")}`,bytes]));
  const variableFontWeight=await readFile(join(root,"crates/valle-compiler/tests/fixtures/motion/composition/variable-font-weight.motion.tsx"),"utf8");
  const exact=variableFontWeight.replace("fps: 30, duration: 2","fps: 16, duration: 1.0625").replace("ctx.progress","ctx.seconds");
  const frames=[...Array.from({length:17},(_,i)=>i),16,0,8,16];
  const cases=[
    {name:"CJK",source:exact.replace(">WEIGHT</Text>",">中文 AB</Text>"),frames:[0,8,16,8],reference:"",monotonic:true},
    {name:"variable-font-weight",source:variableFontWeight,frames:[...Array.from({length:9},(_,i)=>i*7),30,0,56,30],reference:"",monotonic:true},
    {name:"weight",source:exact,frames,reference:"",monotonic:true},
    {name:"axis",source:exact.replace("fontWeight: 100 + ctx.seconds * 800",'fontVariationSettings: `"wght" ${100 + ctx.seconds * 800}`'),frames,reference:"weight",monotonic:true},
    {name:"span",source:exact.replace("fontWeight: 100 + ctx.seconds * 800","fontWeight: 400").replace(">WEIGHT</Text>",'><Span style={{fontVariationSettings:`"wght" ${100 + ctx.seconds * 800}`}}>WEIGHT</Span></Text>'),frames:[0,8,16,8],reference:"weight",monotonic:true},
    {name:"width",source:exact.replace("fontWeight: 100 + ctx.seconds * 800",'fontVariationSettings: `"wght" 500, "wdth" ${62.5 + ctx.seconds * 37.5}`'),frames:[0,4,8,12,16,0],reference:"",monotonic:false},
  ];
  const measuredWidths:number[]=[];
  for (const width of [62.5,75,100]) {
    const compiled=compileMotionJsxWithWasm({compile_motion_jsx},`export const composition={width:640,height:360,duration:1};
      const m=measureText("WEIGHT",{fontSize:80,fontVariationSettings:'"wght" 650.25, "wdth" ${width}'});
      export default function T(){return <View key="measured" style={{width:m.width}}/>;}`,{fonts:[font]});
    const nodes=compiled.artifact.nodes as Array<{styles:Array<{property:string;value:{value:{value:number}}}>}>;
    const binding=nodes.flatMap(n=>n.styles).find(s=>s.property==="width")!;
    measuredWidths.push(binding.value.value.value);
  }
  expect(measuredWidths.every(Number.isFinite)).toBe(true);
  expect(measuredWidths[0]!).toBeLessThan(measuredWidths[1]!);
  expect(measuredWidths[1]!).toBeLessThan(measuredWidths[2]!);

  const dir=await mkdtemp(join(tmpdir(),"valle-variable-fonts-"));
  const spawn=(args:string[])=>Bun.spawn([cli,"--json",...args],{cwd:dir,env:{...process.env,VALLE_HOME:join(dir,"home")},stdout:"pipe",stderr:"pipe"});
  const images=new Map<string,Uint8Array>();
  const info={width:640,height:360,colorType:ck.ColorType.RGBA_8888,alphaType:ck.AlphaType.Unpremul,colorSpace:ck.ColorSpace.SRGB};
  try {
    for (const fixture of cases) {
      await writeFile(join(dir,"scene.motion.tsx"),fixture.source);
      const server=spawn(["motion","studio","scene.motion.tsx","--port","0","--web-assets-dir",join(root,"web/dist")]);
      const surface=ck.MakeSurface(640,360)!;
      let engine:ProductEngine|undefined, executor:CanvasKitExecutor|undefined;
      const stats=new Map<number,{web:Ink;native:Ink}>();
      try {
        const reader=server.stdout.getReader(); let line="";
        try { while(!line.includes("\n")) { const part=await reader.read(); if(part.done)throw new Error(await new Response(server.stderr).text()); line+=new TextDecoder().decode(part.value); } }
        finally { reader.releaseLock(); }
        const ready=JSON.parse(line.split("\n")[0]!);
        const config=await(await fetch(new URL("/config.json",ready.url))).json() as Record<string,string>;
        if(!config.fixedPackageManifestJson)throw new Error(JSON.stringify(config));
        const served=new Uint8Array(await(await fetch(new URL("/runtime/fonts/NotoSans-Variable.ttf",ready.url))).arrayBuffer());
        expect(served).toEqual(font);
        engine=new ProductEngine();
        const receipt=JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!,config.timelineJson!,config.resourceManifestJson!,config.verifiedBindingBundleJson!));
        executor=new CanvasKitExecutor(ck,{transform_srgb_preview_pixels:engine.transform_srgb_preview_pixels.bind(engine),pack_motion_glass_uniforms:engine.pack_motion_glass_uniforms.bind(engine),pack_motion_glass_foreground_uniforms:engine.pack_motion_glass_foreground_uniforms.bind(engine)});
        for (const [request,frame] of fixture.frames.entries()) {
          const ticket=engine.evaluate_prepare_preview(receipt.renderId,BigInt(frame),640,360,false);
          let web:Uint8Array;
          try {
            const requests = await decodePackedAbi(engine.resource_requests(ticket), RESOURCE_REQUESTS_ABI) as
              Array<{ handle: number; key: PackedValue; expected: { kind: string } }>;
            const objects = new Map<number, CanvasKitExternalObject>();
            for (const request of requests) {
              expect(request.expected.kind).toBe("fontBytes");
              const bytes=fontBytes.get(String((request.key as {content:PackedValue}).content));
              if(!bytes)throw new Error(`Unrecognized font: ${JSON.stringify(request.key)}`);
              objects.set(request.handle, { key: request.key, kind: "font", bytes });
            }
            engine.lower_canvas_kit(ticket,64n*1024n*1024n,128n*1024n*1024n); engine.bind(ticket,1n);
            await executor.execute(engine.plan_template_bytes(ticket),engine.binding_bytes(ticket),engine.bound_schedule_bytes(ticket),{generation:1n,objects},{surface});
            web=Uint8Array.from(surface.getCanvas().readPixels(0,0,info)!);
          } finally { engine.release_ticket(ticket); }
          const output=join(dir,`${fixture.name}-${request}.png`);
          const render=spawn(["motion","render","scene.motion.tsx","--frame",String(frame),"--backend","raster","-o",output]);
          const [stdout,stderr,code]=await Promise.all([new Response(render.stdout).text(),new Response(render.stderr).text(),render.exited]);
          if(code!==0)throw new Error(`${stdout}\n${stderr}`);
          const image=ck.MakeImageFromEncoded(await readFile(output))!;
          const native=Uint8Array.from(image.readPixels(0,0,info)!);image.delete();
          let max=0;for(let i=0;i<web.length;i++)max=Math.max(max,Math.abs(web[i]!-native[i]!));
          expect(max,`${fixture.name}/${frame} parity`).toBeLessThanOrEqual(2);
          const key=`${fixture.name}/${frame}`;
          if(images.has(key))expect(web,`${key} random access`).toEqual(images.get(key)!);
          if(fixture.reference)expect(web,`${key} equivalent axis`).toEqual(images.get(`${fixture.reference}/${frame}`)!);
          images.set(key,web); stats.set(frame,{web:ink(web),native:ink(native)});
        }
        for (const backend of ["web","native"] as const) {
          // The extra frame 30 exercises random access; variable-font-weight's acceptance uses exactly 0,7,...,56.
          const ordered=[...stats].filter(([frame])=>fixture.name!=="variable-font-weight" || frame%7===0)
            .sort(([a],[b])=>a-b).map(([,s])=>s[backend]);
          expect(ordered.every(s=>s.count>1000)).toBe(true);
          for(let i=1;i<ordered.length;i++) {
            expect(ordered[i]!.width).toBeGreaterThanOrEqual(ordered[i-1]!.width);
            if(fixture.monotonic) {
              expect(ordered[i]!.count).toBeGreaterThan(ordered[i-1]!.count);
              expect(ordered[i]!.coverage).toBeGreaterThan(ordered[i-1]!.coverage);
            }
          }
          if(fixture.name==="variable-font-weight")expect(new Set(ordered.map(s=>s.width)).size).toBeGreaterThanOrEqual(8);
          if(fixture.name==="weight")expect(new Set(ordered.map(s=>s.width)).size).toBe(17);
          console.info(`${fixture.name}/${backend}`,JSON.stringify(ordered));
        }
      } catch(error) { throw new Error(`${fixture.name}: ${error instanceof Error?error.message:String(error)}`,{cause:error}); }
      finally { server.kill();await server.exited;executor?.dispose();engine?.free();surface.delete(); }
    }
  } finally { await rm(dir,{recursive:true,force:true}); }
},180_000);
