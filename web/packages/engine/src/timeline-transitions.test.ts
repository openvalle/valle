import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine, compile_timeline, timeline_document_view } from "../generated/web/valle_engine.js";
import { CanvasKitExecutor } from "./executor/canvaskit/executor.ts";
import { transitionLabels, setTimelineTransition } from "../../../apps/studio/src/timeline-edit.ts";
import type { Timeline } from "./index.ts";

const root = resolve(import.meta.dir, "../../../../");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
const smooth = (x: number) => { x = Math.max(0, Math.min(1, x)); return x*x*(3-2*x); };
const linear = (n: number) => n/255 <= 0.04045 ? n/255/12.92 : ((n/255+0.055)/1.055)**2.4;
const encoded = (n: number) => Math.round(255*(n <= 0.0031308 ? n*12.92 : 1.055*n**(1/2.4)-0.055));
const blend = (t: number) => encoded(linear(64)*(1-t)+linear(192)*t);

test("all 13 Timeline transitions render exact endpoints and analytic midpoints on Native and CanvasKit", async () => {
  const ck = await CanvasKitInit({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm", import.meta.dir) });
  initSync({ module: await readFile(join(root,"web/packages/engine/generated/web/valle_engine_bg.wasm")) });
  const dir = await mkdtemp(join(tmpdir(),"valle-timeline-transitions-"));
  const spawn = (args: string[]) => Bun.spawn([cli,"--json",...args],{cwd:dir,env:{...process.env,VALLE_HOME:join(dir,"home")},stdout:"pipe",stderr:"pipe"});
  const circleOpenTimeline = JSON.parse(await readFile(join(root,"crates/valle-compiler/tests/fixtures/motion/composition/circle-open-transition.timeline.json"),"utf8")) as Timeline;
  const cases = Object.keys(transitionLabels).flatMap(kind => [false,true].map(empty => ({
    name: `${kind}${empty ? "-transparent" : ""}`, kind, empty, frames: empty ? [3] : [3,1,5,0,6,3],
    timeline: {
      canvas: { width:96,height:64,fps:4,background:"#ffffff" },
      tracks: { visual: [{ clips: [
        {kind:"solid",color:"#404040",start:0,duration:1.5,opacity:empty ? 0 : 1},
        {kind:"solid",color:"#c0c0c0",start:0.25,duration:2,opacity:empty ? 0 : 1},
      ], transitions:[{from:0,to:1,kind}] }] },
    } as Timeline,
  })));
  cases.push({name:"circle-open-transition",kind:"circleOpen",empty:false,frames:[60,44,45,74,76,60],timeline:circleOpenTimeline});
  try {
    for (const fixture of cases) {
      const {width,height} = fixture.timeline.canvas;
      const info = {width,height,colorType:ck.ColorType.RGBA_8888,alphaType:ck.AlphaType.Unpremul,colorSpace:ck.ColorSpace.SRGB};
      await writeFile(join(dir,"timeline.json"),JSON.stringify(fixture.timeline));
      const server = spawn(["timeline","studio","timeline.json","--port","0","--web-assets-dir",join(root,"web/dist")]);
      const surface = ck.MakeSurface(width,height)!;
      let engine: ProductEngine | undefined, executor: CanvasKitExecutor | undefined;
      try {
        const reader = server.stdout.getReader();
        let line = "";
        try {
          while (!line.includes("\n")) {
            const part = await reader.read();
            if (part.done) throw new Error(await new Response(server.stderr).text());
            line += new TextDecoder().decode(part.value);
          }
        } finally { reader.releaseLock(); }
        const ready = JSON.parse(line.split("\n")[0]!);
        const config = await (await fetch(new URL("/config.json",ready.url))).json() as Record<string,string>;
        if (!config.fixedPackageManifestJson) throw new Error(JSON.stringify(config));
        engine = new ProductEngine();
        const receipt = JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!,config.timelineJson!,config.resourceManifestJson!,config.verifiedBindingBundleJson!));
        executor = new CanvasKitExecutor(ck,{
          transform_srgb_preview_pixels:engine.transform_srgb_preview_pixels.bind(engine),
          pack_motion_glass_uniforms:engine.pack_motion_glass_uniforms.bind(engine),
          pack_motion_glass_foreground_uniforms:engine.pack_motion_glass_foreground_uniforms.bind(engine),
        });
        let first: Uint8Array | undefined;
        for (const [request,frame] of fixture.frames.entries()) {
          const ticket = engine.evaluate_prepare_preview(receipt.renderId,BigInt(frame),width,height,false);
          let web: Uint8Array;
          try {
            engine.lower_canvas_kit(ticket,64n*1024n*1024n,128n*1024n*1024n); engine.bind(ticket,1n);
            await executor.execute(engine.plan_template_bytes(ticket),engine.binding_bytes(ticket),engine.bound_schedule_bytes(ticket),{generation:1n,objects:new Map()},{surface});
            web = Uint8Array.from(surface.getCanvas().readPixels(0,0,info)!);
          } finally { engine.release_ticket(ticket); }
          const output = join(dir,`${fixture.name}-${request}.png`);
          const nativeRun = spawn(["timeline","render","timeline.json","--frame",String(frame),"-o",output]);
          const [stdout,stderr,code] = await Promise.all([new Response(nativeRun.stdout).text(),new Response(nativeRun.stderr).text(),nativeRun.exited]);
          if (code !== 0) throw new Error(`${stdout}\n${stderr}`);
          const image = ck.MakeImageFromEncoded(await readFile(output))!;
          const native = Uint8Array.from(image.readPixels(0,0,info)!); image.delete();
          let maximum = 0;
          for (let i=0;i<web.length;i++) maximum = Math.max(maximum,Math.abs(web[i]!-native[i]!));
          expect(maximum,`${fixture.name}/${frame} parity`).toBeLessThanOrEqual(2);
          if (request === 0) first = web;
          if (request === fixture.frames.length-1) expect(web,`${fixture.name} random seek`).toEqual(first!);
          for (const pixels of [web,native]) {
            const probe = (x:number,y:number,expected:readonly number[]) => {
              for (let c=0;c<3;c++) expect(Math.abs(pixels[(y*width+x)*4+c]!-expected[c]!),`${fixture.name}/${frame} (${x},${y})/${c}`).toBeLessThanOrEqual(2);
            };
            if (fixture.name === "circle-open-transition") {
              probe(320,180,frame<60 ? [33,64,255] : [255,74,36]);
              probe(0,0,frame<74 ? [33,64,255] : [255,74,36]);
              if (frame===60) {
                // Equal physical radii on a wide canvas; normalized UV distance makes an ellipse.
                probe(495,180,[255,74,36]); probe(320,5,[255,74,36]);
                probe(540,180,[33,64,255]);
              }
            } else if (fixture.empty || frame !== 3) {
              const n = fixture.empty ? 255 : frame<=1 ? 64 : 192;
              for (let y=0;y<height;y+=5) for (let x=0;x<width;x+=5) probe(x,y,[n,n,n]);
            } else {
              for (const [x,y] of [[0,0],[24,32],[72,32]] as const) {
                const u=(x+0.5)/width,v=(y+0.5)/height;
                let amount=0.5;
                switch(fixture.kind) {
                  case "wipeLeft": amount=u<0.5?1:0; break;
                  case "wipeRight": amount=u>0.5?1:0; break;
                  case "circleOpen": amount=Math.hypot(x+0.5-width/2,y+0.5-height/2)<Math.hypot(width,height)/4?1:0; break;
                  case "simpleZoom": amount=0; break; // Fade starts at quickness - 0.2 = 0.6.
                  case "crossWarp": amount=smooth(u); break;
                  case "directionalWarp": amount=1-smooth(-u+v+0.5); break;
                  case "ripple": amount=smooth(0.3/0.8); break;
                  case "perlin": if(x!==0) continue; amount=1; break;
                }
                const n=fixture.kind==="multiplyBlend" ? encoded(linear(64)*linear(192)) : blend(amount);
                probe(x,y,[n,n,n]);
              }
            }
          }
        }
      } catch(error) { throw new Error(`${fixture.name}: ${error instanceof Error?error.message:String(error)}`,{cause:error}); }
      finally { server.kill(); await server.exited; executor?.dispose(); engine?.free(); surface.delete(); }
    }
    const edited = setTimelineTransition(circleOpenTimeline,"/tracks/visual/0/clips/0","wipeRight",0.5);
    const projected=JSON.parse(timeline_document_view(compile_timeline(JSON.stringify(edited),"{}")));
    expect(projected.sequences[0].items.map((i:any)=>[i.startFrame,i.endFrame])).toEqual([[0,75],[60,75],[60,135]]);
  } finally { await rm(dir,{recursive:true,force:true}); }
},180_000);
