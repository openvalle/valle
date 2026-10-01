import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../generated/web/valle_engine.js";
import { CanvasKitExecutor } from "./executor/canvaskit/executor.ts";
import { transitionLabels } from "../../../apps/studio/src/timeline-edit.ts";

const root = resolve(import.meta.dir, "../../../../");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
type Fixture = { name:string; command:"motion"|"timeline"; content:string; width:number; height:number; frames:number[]; reference?:string; referenceOffset?:number; different?:string; probe?:(pixels:Uint8Array,frame:number)=>void };
const motion = (body:string) => `export const composition={width:96,height:64,fps:4,duration:1.25};
export default function T(ctx){return <Scene style={{width:96,height:64,backgroundColor:"#ffffff"}}>${body}</Scene>;}`;
const view = (color:string,extra="") => `<View style={{width:96,height:64,backgroundColor:"${color}"${extra}}}/>`;
const transition = (kind:string,from:string,to:string,extra="",progress="ctx.seconds") => `<Transition kind="${kind}" progress={${progress}} style={{width:96,height:64${extra}}}>${from}${to}</Transition>`;
const diff = (a:Uint8Array,b:Uint8Array) => { let max=0; expect(a.length).toBe(b.length); for(let i=0;i<a.length;i++) max=Math.max(max,Math.abs(a[i]!-b[i]!));return max; };
const pixel = (pixels:Uint8Array,width:number,x:number,y:number,rgba:number[]) => {
  for(const [c,value] of rgba.entries()) expect(Math.abs(pixels[(y*width+x)*4+c]!-value)).toBeLessThanOrEqual(2);
};

test("Motion Transition shares all Timeline kernels and preserves nested subtree semantics across backends", async () => {
  const ck = await CanvasKitInit({ locateFile:()=>Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm",import.meta.dir) });
  initSync({module:await readFile(join(root,"web/packages/engine/generated/web/valle_engine_bg.wasm"))});
  const dir = await mkdtemp(join(tmpdir(),"valle-motion-transitions-"));
  const spawn = (args:string[]) => Bun.spawn([cli,"--json",...args],{cwd:dir,env:{...process.env,VALLE_HOME:join(dir,"home")},stdout:"pipe",stderr:"pipe"});
  const cases: Fixture[] = [];
  const parameters: Record<string, Record<string, number>> = {
    wipeLeft:{softness:0.25}, wipeRight:{softness:0.25}, circleOpen:{centerX:0.2,centerY:0.3,softness:4},
    simpleZoom:{quickness:0.4,centerX:0.2,centerY:0.7}, crossWarp:{centerX:0.2,centerY:0.7},
    linearBlur:{intensity:0.8}, directionalWarp:{directionX:0.6,directionY:-0.5,softness:0.2},
    dreamyZoom:{strength:1.5,centerX:0.2,centerY:0.7}, ripple:{frequency:20,speed:12,amplitude:0.2},
    flyEye:{size:0.2,frequency:8,colorSeparation:0.8}, multiplyBlend:{midpoint:0.2}, perlin:{scale:12,smoothness:0.03},
  };
  const patternedFrom = `<View style={{position:'absolute',width:64,height:48,left:12,top:8,backgroundImage:"linear-gradient(90deg in srgb, red, blue)",opacity:0.6,borderRadius:12}}/>`;
  const patternedTo = `<Mask rect={rect(0,0,96,64)} style={{width:96,height:64}}><MaskSource><Circle cx={48} cy={32} r={22} fill="#fff"/></MaskSource>${view("#00ff00")}</Mask>`;
  const sourceClip = (body:string) => `export const composition={width:96,height:64,fps:4,duration:3}; export default function T(){return <Scene style={{width:96,height:64}}>${body}</Scene>;}`;
  await writeFile(join(dir,"param-from.motion.tsx"),sourceClip(patternedFrom));
  await writeFile(join(dir,"param-to.motion.tsx"),sourceClip(patternedTo));
  for(const kind of Object.keys(transitionLabels)) {
    const name = `motion-${kind}`;
    cases.push({name,command:"motion",content:motion(transition(kind,view("#404040"),view("#c0c0c0"))),width:96,height:64,frames:[2,0,4,2]});
    cases.push({name:`timeline-${kind}`,command:"timeline",content:JSON.stringify({
      canvas:{width:96,height:64,fps:4,background:"#ffffff"},tracks:{visual:[{clips:[
        {kind:"solid",color:"#404040",start:0,duration:1.5},
        {kind:"solid",color:"#c0c0c0",start:0.25,duration:2},
      ],transitions:[{from:0,to:1,kind}]}]},
    }),width:96,height:64,frames:[3,1,5,3],reference:name,referenceOffset:-1});
    cases.push({name:`empty-${kind}`,command:"motion",content:motion(transition(kind,"<View/>","<View/>")),width:96,height:64,frames:[2,0,4],probe:(p)=>pixel(p,96,48,32,[255,255,255,255])});
    const from = patternedFrom, to = patternedTo;
    cases.push({name:`pattern-${kind}`,command:"motion",content:motion(transition(kind,from,to)),width:96,height:64,frames:[2,0,4,2],reference:kind==="fade" ? undefined:"pattern-fade"});
    const params = parameters[kind];
    if (params) {
      const parameterized = transition(kind,from,to).replace(" progress=", ` params={${JSON.stringify(params)}} progress=`);
      cases.push({name:`param-motion-${kind}`,command:"motion",content:motion(parameterized),width:96,height:64,frames:[2,0,4,2],different:`pattern-${kind}`});
      cases.push({name:`param-timeline-${kind}`,command:"timeline",content:JSON.stringify({
        canvas:{width:96,height:64,fps:4,background:"#ffffff"}, resources:{from:"param-from.motion.tsx",to:"param-to.motion.tsx"},
        tracks:{visual:[{clips:[
          {kind:"motion",component:"from",start:0,duration:1.5},
          {kind:"motion",component:"to",start:0.25,duration:2},
        ],transitions:[{from:0,to:1,kind,params}]}]},
      }),width:96,height:64,frames:[3,1,5,3],reference:`param-motion-${kind}`,referenceOffset:-1});
      // Animate the first parameter at fixed transition progress to catch stale geometry/schedules.
      const [animated, value] = Object.entries(params)[0]!;
      const expression = JSON.stringify(params).replace(`"${animated}":${value}`, `"${animated}":${value}*(0.5+ctx.seconds)`);
      cases.push({name:`param-dynamic-${kind}`,command:"motion",content:motion(transition(kind,from,to,"","0.5").replace(" progress=",` params={${expression}} progress=`)),width:96,height:64,frames:[2,0,4,2],probe:(pixels,frame)=>{
        if(frame===2)expect(diff(pixels,images.get(`param-motion-${kind}/2`)!)).toBeLessThanOrEqual(2);
        if(frame===4)expect(diff(pixels,images.get(`param-dynamic-${kind}/0`)!),`${kind} animated parameter changes pixels`).toBeGreaterThan(2);
      }});
    }
    for (const customized of params ? [false,true] : [false]) {
      let body = transition(kind,from,to,"","ctx.seconds < 0.5 ? 0.000001 : 0.999999");
      if (customized) body = body.replace(" progress=", ` params={${JSON.stringify(params)}} progress=`);
      cases.push({name:`edge-${customized?"param-":""}${kind}`,command:"motion",content:motion(body),
        width:96,height:64,frames:[0,4],reference:customized?`param-motion-${kind}`:`pattern-${kind}`});
    }
  }
  // Endpoint identity must hold for any kernel; intermediate patterns deliberately differ.
  const motionTransition = await readFile(join(root,"crates/valle-compiler/tests/fixtures/motion/composition/motion-transition.motion.tsx"),"utf8");
  cases.push({name:"motion-transition",command:"motion",content:motionTransition,width:640,height:360,frames:[30,0,59,30],probe:(p,f)=>{
    pixel(p,640,320,180,f===0?[33,64,255,255]:[255,74,36,255]);
    pixel(p,640,0,0,[33,64,255,255]);
  }});
  cases.push({name:"motion-transition-timeline",command:"timeline",content:JSON.stringify({canvas:{width:640,height:360,fps:30,background:"#101010"},tracks:{visual:[{clips:[
    {kind:"solid",color:"#2140ff",start:0,duration:91/30},{kind:"solid",color:"#ff4a24",start:1,duration:3},
  ],transitions:[{from:0,to:1,kind:"circleOpen"}]}]}}),width:640,height:360,frames:[60,30,89,60],reference:"motion-transition",referenceOffset:-30});
  const extra: Array<[string,string,(p:Uint8Array,f:number)=>void]> = [
    ["simpleZoom-settles",transition("simpleZoom",patternedFrom,patternedTo,"","0.9999"),(p)=>{
      pixel(p,96,48,32,[0,255,0,255]);pixel(p,96,0,0,[255,255,255,255]);pixel(p,96,95,63,[255,255,255,255]);
    }],
    ["z-order",transition("fade",view("#404040",",zIndex:5"),view("#c0c0c0",",zIndex:-5")),(p,f)=>pixel(p,96,48,32,[...Array(3).fill(f===0?64:f===4?192:146),255])],
    ["hidden",transition("fade",'<View style={{display:"none"}}/>',view("#000000")),(p,f)=>pixel(p,96,48,32,[...Array(3).fill(f===0?255:f===4?0:188),255])],
    ["inactive",transition("fade",'<View visible={false}/>',view("#000000")),(p,f)=>pixel(p,96,48,32,[...Array(3).fill(f===0?255:f===4?0:188),255])],
    ["nested",transition("fade",transition("fade",view("#000000"),view("#ffffff"),"","0.5"),view("#000000")),(p,f)=>pixel(p,96,48,32,[...Array(3).fill(f===0?188:f===4?0:137),255])],
    ["owner-offset",transition("fade",view("#000000"),view("#000000"),",position:'absolute',left:24,top:16,width:48,height:32,opacity:0.5"),(p)=>{pixel(p,96,30,30,[188,188,188,255]);pixel(p,96,1,1,[255,255,255,255]);}],
    ["semantic-isolation",transition("fade",view("#000000",",isolation:'auto'"),view("#ffffff",",isolation:'auto'"),",isolation:'auto'"),(p,f)=>pixel(p,96,48,32,[...Array(3).fill(f===0?0:f===4?255:188),255])],
    ["input-destination",transition("fade",view("#ff0000"),view("#404040",",mixBlendMode:'screen'")),(p,f)=>{if(f===4)pixel(p,96,48,32,[64,64,64,255]);}],
    ["owner-transform",transition("ripple",view("#404040"),view("#c0c0c0"),",transform:'rotate(17deg) scale(0.6,0.75)'"),()=>{}],
    ["owner-filter",transition("fade",view("#404040"),view("#c0c0c0"),",filter:'blur(3px)'"),()=>{}],
    ["owner-clip",transition("fade",view("#000000"),view("#000000"),",borderRadius:32,overflow:'hidden'"),(p)=>{pixel(p,96,48,32,[0,0,0,255]);pixel(p,96,0,0,[255,255,255,255]);}],
  ];
  for(const [name,body,probe] of extra) cases.push({name,command:"motion",content:motion(body),width:96,height:64,frames:[2,0,4,2],probe});
  const filter = process.env.VALLE_TEST_TRANSITION_CASE;
  const selected = new Set(cases.filter(c=>!filter || new RegExp(filter).test(c.name)).map(c=>c.name));
  for (const fixture of [...cases].reverse()) if (selected.has(fixture.name)) {
    if (fixture.reference) selected.add(fixture.reference);
    if (fixture.different) selected.add(fixture.different);
    if (fixture.name.startsWith("param-dynamic-")) selected.add(fixture.name.replace("param-dynamic-", "param-motion-"));
  }
  const images = new Map<string,Uint8Array>();
  const parityFailures: Array<{fixture:string; frame:number; delta:number}> = [];
  try {
    for(const fixture of cases.filter(c=>selected.has(c.name))) {
      const {width,height}=fixture;
      const info={width,height,colorType:ck.ColorType.RGBA_8888,alphaType:ck.AlphaType.Unpremul,colorSpace:ck.ColorSpace.SRGB};
      const entry=fixture.command==="motion"?"scene.motion.tsx":"timeline.json";
      await writeFile(join(dir,entry),fixture.content);
      const server=spawn([fixture.command,"studio",entry,"--port","0","--web-assets-dir",join(root,"web/dist")]);
      const surface=ck.MakeSurface(width,height)!;
      let engine:ProductEngine|undefined,executor:CanvasKitExecutor|undefined;
      try {
        const reader=server.stdout.getReader();let line="";
        try {while(!line.includes("\n")){const part=await reader.read();if(part.done)throw new Error(await new Response(server.stderr).text());line+=new TextDecoder().decode(part.value);}}
        finally {reader.releaseLock();}
        const ready=JSON.parse(line.split("\n")[0]!);
        const config=await(await fetch(new URL("/config.json",ready.url))).json() as Record<string,string>;
        if(!config.fixedPackageManifestJson)throw new Error(JSON.stringify(config.diagnostics ?? config));
        engine=new ProductEngine();
        const receipt=JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!,config.timelineJson!,config.resourceManifestJson!,config.verifiedBindingBundleJson!));
        executor=new CanvasKitExecutor(ck,{transform_srgb_preview_pixels:engine.transform_srgb_preview_pixels.bind(engine),pack_motion_glass_uniforms:engine.pack_motion_glass_uniforms.bind(engine),pack_motion_glass_foreground_uniforms:engine.pack_motion_glass_foreground_uniforms.bind(engine)});
        for(const [request,frame] of fixture.frames.entries()) {
          const ticket=engine.evaluate_prepare_preview(receipt.renderId,BigInt(frame),width,height,false);
          let web:Uint8Array;
          try {
            engine.lower_canvas_kit(ticket,64n*1024n*1024n,128n*1024n*1024n);engine.bind(ticket,1n);
            await executor.execute(engine.plan_template_bytes(ticket),engine.binding_bytes(ticket),engine.bound_schedule_bytes(ticket),{generation:1n,objects:new Map()},{surface});
            web=Uint8Array.from(surface.getCanvas().readPixels(0,0,info)!);
          } finally {engine.release_ticket(ticket);}
          const output=join(dir,`${fixture.name}-${request}.png`);
          const nativeRun=spawn([fixture.command,"render",entry,"--frame",String(frame),...(fixture.command==="motion"?["--backend","raster"]:[]),"-o",output]);
          const [stdout,stderr,code]=await Promise.all([new Response(nativeRun.stdout).text(),new Response(nativeRun.stderr).text(),nativeRun.exited]);
          if(code!==0)throw new Error(`${stdout}\n${stderr}`);
          const image=ck.MakeImageFromEncoded(await readFile(output))!;
          const native=Uint8Array.from(image.readPixels(0,0,info)!);image.delete();
          const parity=diff(web,native);
          if(parity>2) {
            const points=[];
            for(let i=0;i<web.length;i++) if(Math.abs(web[i]!-native[i]!)>2) points.push({x:Math.floor(i/4)%width,y:Math.floor(i/4/width),c:i%4,web:web[i],native:native[i]});
            console.info(fixture.name,JSON.stringify(points.slice(0,12)),"count",points.length);
            parityFailures.push({fixture:fixture.name,frame,delta:parity});
          }
          fixture.probe?.(web,frame);fixture.probe?.(native,frame);
          const key=`${fixture.name}/${frame}`;
          if(images.has(key))expect(web,`${key} random access`).toEqual(images.get(key)!);
          images.set(key,web);
          if(fixture.different) {
            const delta = diff(web,images.get(`${fixture.different}/${frame}`)!);
            if(frame===2) expect(delta,`${key} parameter changes pixels`).toBeGreaterThan(2);
            else expect(delta,`${key} exact endpoint`).toBeLessThanOrEqual(2);
          }
          if(fixture.reference && (!fixture.name.startsWith("pattern-")||frame!==2)) {
            expect(diff(web,images.get(`${fixture.reference}/${frame+(fixture.referenceOffset??0)}`)!),`${key} reference`).toBeLessThanOrEqual(2);
          }
        }
      } catch(error) {throw new Error(`${fixture.name}: ${error instanceof Error?error.message:String(error)}`,{cause:error});}
      finally {server.kill();await server.exited;executor?.dispose();engine?.free();surface.delete();}
    }
    expect(parityFailures, "Native / CanvasKit channel differences must not exceed 2").toEqual([]);
  } finally {await rm(dir,{recursive:true,force:true});}
},360_000);
