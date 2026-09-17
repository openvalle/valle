import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../generated/web/valle_engine.js";
import { decodePackedAbi, RESOURCE_REQUESTS_ABI } from "./abi/packed.ts";

const root = resolve(import.meta.dir, "../../../..");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
// Run with VALLE_TEST_NATIVE_BACKEND=metal outside the sandbox to exercise the GPU.
const nativeBackend = process.env.VALLE_TEST_NATIVE_BACKEND ?? "raster";
if (nativeBackend !== "raster" && nativeBackend !== "metal") throw new Error("unsupported test backend");

function concatBytes(...chunks: Uint8Array[]): Uint8Array {
  const bytes = new Uint8Array(chunks.reduce((size, chunk) => size + chunk.length, 0));
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return bytes;
}

test("project environments and models survive frozen-package reopen and arbitrary frame access", async () => {
  const { default: CanvasKitInit } = await import("canvaskit-wasm/full") as unknown as {
    default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
  };
  const ck = await CanvasKitInit({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/full/canvaskit.wasm", import.meta.dir) });
  initSync({ module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)) });
  const dir = await mkdtemp(join(tmpdir(), "valle-scene3d-parity-"));
  const info = { width:64,height:64,colorType:ck.ColorType.RGBA_8888,alphaType:ck.AlphaType.Unpremul,colorSpace:ck.ColorSpace.SRGB };
  const spawn = (args: string[]) => Bun.spawn([cli, "--json", ...args], {
    cwd:dir,env:{...process.env,VALLE_HOME:join(dir,"home")},stdout:"pipe",stderr:"pipe",
  });
  try {
    for (const kind of ["unlit", "pbr", "environment", "hierarchy", "mask", "animated", "material"]) {
      const environment = kind === "environment";
      const animated = kind === "animated";
      const material = kind === "material";
      const unlit = kind === "unlit" || kind === "hierarchy";
      await copyFile(join(root, "crates/valle-motion/tests/fixtures/scene3d", unlit || kind === "mask" ? "triangle.glb" : "pbr.glb"),join(dir,"model.glb"));
      if (kind === "hierarchy" || kind === "mask") {
        const bytes = await readFile(join(dir,"model.glb"));
        const jsonLength = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(12, true);
        const model = JSON.parse(new TextDecoder().decode(bytes.subarray(20,20+jsonLength)));
        let bin: Uint8Array = bytes.subarray(28+jsonLength,28+jsonLength+model.buffers[0].byteLength);
        if (kind === "hierarchy") {
        model.nodes = [
          {children:[2],scale:[1.2,0.8,1]},
          {mesh:0,matrix:[-0.35,0,0,0,0,0.6,0,0,0,0,1,0,1.0,0,0,1]},
          {mesh:0,name:"part",scale:[0.7,1.1,1]},
        ];
        model.scenes[model.scene].nodes = [0,1];
        } else {
          const image = ck.MakeImage({...info,width:2,height:1},new Uint8Array([255,255,255,0,255,255,255,255]),8)!;
          let png: Uint8Array;
          try { png=Uint8Array.from(image.encodeToBytes()!); } finally {image.delete();}
          const padding = new Uint8Array((4-bin.length%4)%4);
          const offset = bin.length+padding.length;
          bin = concatBytes(bin,padding,png);
          model.images=[{bufferView:model.bufferViews.length,mimeType:"image/png"}];
          model.bufferViews.push({buffer:0,byteOffset:offset,byteLength:png.length});
          model.textures=[{source:0,sampler:0}];
          model.samplers=[{minFilter:9728,magFilter:9728}];
          model.materials=[
            {alphaMode:"MASK",doubleSided:true,emissiveFactor:[0.8,0.1,0.1],pbrMetallicRoughness:{baseColorTexture:{index:0}},occlusionTexture:{index:0}},
            {emissiveFactor:[0.1,0.8,0.1]},
          ];
          model.meshes[0].primitives[0].material=0;
          const back=structuredClone(model.meshes[0]); back.primitives[0].material=1;
          model.meshes.push(back);
          model.nodes=[{mesh:0},{mesh:1,translation:[0,0,-0.5],scale:[1.2,1.2,1]}];
          model.scenes[model.scene].nodes=[0,1];
        }
        model.buffers[0].byteLength=bin.length;
        // Freeze hierarchy data into the same ordinary GLB asset path used by every model.
        const json = new TextEncoder().encode(JSON.stringify(model));
        const padded = new Uint8Array(Math.ceil(json.length/4)*4).fill(0x20); padded.set(json);
        const binPadding = new Uint8Array((4-bin.length%4)%4);
        const binHeader = new Uint8Array(8);
        const binView = new DataView(binHeader.buffer);
        binView.setUint32(0,bin.length+binPadding.length,true); binView.setUint32(4,0x004e4942,true);
        const binChunk = concatBytes(binHeader,bin,binPadding);
        const header = new Uint8Array(20);
        const headerView = new DataView(header.buffer);
        headerView.setUint32(0,0x46546c67,true); headerView.setUint32(4,2,true);
        headerView.setUint32(8,20+padded.length+binChunk.length,true);
        headerView.setUint32(12,padded.length,true); headerView.setUint32(16,0x4e4f534a,true);
        await writeFile(join(dir,"model.glb"),concatBytes(header,padded,binChunk));
      }
      const pixels = new Uint8Array(16*8*4);
      for (let i=0;i<128;i++) pixels.set(i%16<8 ? [255,32,0,255] : [0,32,255,255],i*4);
      const image = ck.MakeImage({...info,width:16,height:8},pixels,16*4)!;
      try { await writeFile(join(dir,"sky.png"),image.encodeToBytes()!); } finally { image.delete(); }
      const textureImage = ck.MakeImage({...info,width:2,height:2},new Uint8Array([128,128,255,255,192,64,128,255,128,255,128,255,255,128,128,255]),8)!;
      let textureBytes:Uint8Array;
      try {textureBytes=Uint8Array.from(textureImage.encodeToBytes()!);} finally {textureImage.delete();}
      await writeFile(join(dir,"map.png"),textureBytes);
      await writeFile(join(dir,"scene.motion.tsx"), `
        export const composition = { width: 64, height: 64, duration: 3 };
        export const controls=defineControls({assets:{model:asset({kind:"model3d"})${environment?',sky:asset({kind:"environment"})':''}${material?',map:asset({kind:"image"})':''}}});
        export default function T(ctx){return <Scene style={{width:64,height:64}}>
          <Scene3D key="test" camera={{${animated ? 'position:[ctx.seconds*0.1,0.1,3+ctx.seconds*0.1],target:[ctx.seconds*0.05,0,0],near:0.1+ctx.seconds*0.01,far:10+ctx.seconds,fov:45-ctx.seconds,orbitYaw:ctx.seconds*10,orbitPitch:ctx.seconds*2,distance:3.1+ctx.seconds*0.1' : 'position:[0,0,3],target:[0,0,0],fov:45'}}} style={{width:64,height:64}}
            pbr={{toneMapping:"${unlit ? "none" : "aces"}"${animated ? ",exposure:0.7+ctx.seconds*0.2" : ""}${environment?',environment:{src:"asset://sky",rotation:ctx.seconds*90,intensity:1,background:true}':''}}}>
            <Mesh key="tri" src="asset://model" rotateY={ctx.seconds*20}
              ${animated ? 'position={[ctx.seconds*0.01,0,0]} translateX={0.1} rotation={[0,5,0]} scale={[1,1,1]} scaleX={1+ctx.seconds*0.01} nodes={[{id:0,position:[ctx.seconds*0.02,0,0],rotation:[0,ctx.seconds*5,0],scale:[-1,1,1]}]}' : kind === 'hierarchy' ? 'nodes={[{id:0,position:[ctx.seconds*0.05,0,0],rotation:[0,ctx.seconds*5,0],scale:[1.2,0.8,1]}]}' : ''} material={{type:"${unlit ? "unlit" : "pbr"}",color:"#ffffff"${material ? ',roughness:0.3+ctx.seconds*0.2,metallic:ctx.seconds*0.3,textures:{baseColor:"asset://map",metallicRoughness:{src:"asset://map",wrapU:"repeat",wrapV:"mirror",minFilter:"nearest",magFilter:"nearest",mipmap:"nearest"},normal:"asset://map",occlusion:"asset://map",emissive:"asset://map"}' : ''}}}
              ${material ? 'materials={[{id:0,color:interpolate(ctx.seconds,[0,2],["#ffaaaa","#aaaaff"]),emissive:"#ffffff",emissiveIntensity:0.1+ctx.seconds*0.2,normalScale:0.2+ctx.seconds*0.2,occlusionStrength:ctx.seconds*0.3,alphaMode:"mask",alphaCutoff:ctx.seconds*0.1,doubleSided:true}]}' : ''}/>

            ${kind === 'pbr' || material ? '<AmbientLight intensity={1}/>' : ''}
            ${animated ? `<AmbientLight color={interpolate(ctx.seconds,[0,2],["#ff0000","#0000ff"])} intensity={0.5+ctx.seconds*0.2}/>
              <DirectionalLight color={interpolate(ctx.seconds,[0,2],["#00ff00","#ff0000"])} direction={[ctx.seconds*0.2,0,1]} intensity={0.5}/>
              <HemisphereLight skyColor="#bad6ff" groundColor="#222222" direction={[ctx.seconds*0.1,1,0]} intensity={0.1}/>` : ''}
          </Scene3D>
        </Scene>; }
      `);
      const assets = ["--asset","model=model.glb",...(environment ? ["--asset","sky=sky.png"] : []),...(material ? ["--asset","map=map.png"] : []),"--fps","30"];
      const server = spawn(["motion","studio","scene.motion.tsx",...assets,"--port","0","--web-assets-dir",join(root,"web/dist")]);
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
        if (!ready.url) throw new Error(JSON.stringify(ready));
        const config = await (await fetch(new URL("/config.json",ready.url))).json() as Record<string,string>;
        const open = () => {
          const engine = new ProductEngine();
          const receipt = JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!,config.timelineJson!,config.resourceManifestJson!,config.verifiedBindingBundleJson!));
          return {engine,renderId:receipt.renderId as string};
        };
        const render = async (engine:ProductEngine,renderId:string,frame:number) => {
          const ticket = engine.evaluate_prepare_preview(renderId,BigInt(frame),64,64,false);
          try {
            const requests = await decodePackedAbi(engine.resource_requests(ticket),RESOURCE_REQUESTS_ABI) as any[];
            const request = requests.find(r=>r.expected.kind==='scene3d')!;
            expect(request).toBeDefined();
            const payload = Uint8Array.from(request.payload.canonical_request as number[]);
            if (animated) {
              const resolved = JSON.parse(new TextDecoder().decode(payload));
              const seconds=frame/30;
              expect(resolved.scene.camera).toBeUndefined();
              expect(resolved.scene.meshes[0].transform).toBeUndefined();
              expect(resolved.scene.meshes[0].nodeIds).toEqual([0]);
              expect(resolved.frame.meshes[0].transform.translation[0]).toBeCloseTo(0.1+seconds*0.01,6);
              expect(resolved.frame.meshes[0].transform.rotationDegrees).toEqual([0,5+seconds*20,0]);
              expect(resolved.frame.meshes[0].transform.scale[0]).toBeCloseTo(1+seconds*0.01,6);
              expect(resolved.frame.meshes[0].nodes[0].transform.translation[0]).toBeCloseTo(seconds*0.02,6);
              expect(resolved.frame.meshes[0].nodes[0].transform.rotationDegrees).toEqual([0,seconds*5,0]);
              expect(resolved.frame.meshes[0].nodes[0].transform.scale).toEqual([-1,1,1]);
              expect(resolved.scene.lights).toEqual(["ambient","directional","hemisphere"]);
              expect(resolved.frame.camera.orbitYawDegrees).toBeUndefined();
              expect(resolved.frame.camera.near).toBeCloseTo(0.1+seconds*0.01,6);
              expect(resolved.frame.camera.far).toBe(10+seconds);
              expect(resolved.frame.camera.target[0]).toBeCloseTo(seconds*0.05,6);
              expect(resolved.frame.camera.fovYDegrees).toBe(45-seconds);
              expect(resolved.frame.exposure).toBeCloseTo(0.7+seconds*0.2,6);
              expect(resolved.frame.lights[1].direction[0]).toBeCloseTo(seconds*0.2,6);
              expect(resolved.frame.lights[2].direction[0]).toBeCloseTo(seconds*0.1,6);
              if (frame===60) expect(resolved.frame.lights[0].color).toEqual([0,0,1,1]);
              else if (frame===0) expect(resolved.frame.lights[0].color).toEqual([1,0,0,1]);
            }
            if (material) {
              const resolved=JSON.parse(new TextDecoder().decode(payload));
              expect(resolved.scene.meshes[0].material.color).toBeUndefined();
              expect(resolved.frame.meshes[0].material.roughness).toBeCloseTo(0.3+frame/30*0.2,6);
              expect(resolved.frame.meshes[0].materialOverrides[0].id).toBe(0);
              expect(resolved.frame.meshes[0].materialOverrides[0].material.emissiveIntensity).toBeCloseTo(0.1+frame/30*0.2,6);
            }
            const needs = JSON.parse(engine.scene3d_resource_needs_json(payload));
            for (const digest of needs.models) engine.register_scene3d_model(digest,engine.compiled_resource_bytes(renderId,"model3d-bytes",digest));
            for (const digest of needs.environments) engine.register_scene3d_environment(digest,engine.compiled_resource_bytes(renderId,"environment-bytes",digest));
            if (needs.textures.length) expect(needs.textures.map((t:any)=>t.role)).toEqual(["color","data"]);
            for (const {digest,role} of needs.textures) engine.register_scene3d_texture(digest,textureBytes,role);
            expect(JSON.parse(engine.scene3d_resource_needs_json(payload))).toEqual({models:[],textures:[],environments:[]});
            const pixels = engine.render_scene3d_request(request.key.content,request.key.interpretation.topology_digest,payload);
            expect(pixels.length).toBe(64*64*4);
            const pick = JSON.parse(engine.scene3d_pick_json(request.key.content,32,32));
            expect(pick.semanticAddress).toBe("test::tri");
            if (kind === "mask") {
              expect([0,1]).toContain(pick.nodeId);
              if (frame === 0) {
                expect(JSON.parse(engine.scene3d_pick_json(request.key.content,26,32)).nodeId).toBe(1);
                expect(JSON.parse(engine.scene3d_pick_json(request.key.content,38,32)).nodeId).toBe(0);
              }
            } else expect(pick.nodeId).toBe(kind === "hierarchy" ? 2 : 0);
            return {pixels,request};
          } finally { engine.release_ticket(ticket); }
        };
        const {engine,renderId} = open();
        try {
          const native = new Map<number,Uint8Array<ArrayBuffer>>();
          for (const [index,frame] of [60,0,30,60].entries()) {
            const name = `${kind}-${index}-${frame}.png`;
            const child = spawn(["motion","render","scene.motion.tsx",...assets,"--frame",String(frame),"--backend",nativeBackend,"-o",name]);
            const [out,err,code] = await Promise.all([new Response(child.stdout).text(),new Response(child.stderr).text(),child.exited]);
            if (code !== 0) throw new Error(`${out}\n${err}`);
            expect(JSON.parse(out).delivery.backend).toBe(nativeBackend);
            const image = ck.MakeImageFromEncoded(await readFile(join(dir,name)))!;
            try {
              const pixels = Uint8Array.from(image.readPixels(0,0,info)!);
              const prior = native.get(frame);
              if (prior) expect(pixels,`${nativeBackend}/${kind}: repeated frame ${frame}`).toEqual(prior);
              native.set(frame,pixels);
            } finally { image.delete(); }
          }
          // Reopen uses the frozen package plus the host's captured, digest-verified image bytes.
          await rm(join(dir,"model.glb")); await rm(join(dir,"sky.png")); await rm(join(dir,"map.png"));
          const frames = new Map<number,Awaited<ReturnType<typeof render>>>();
          for (const frame of [60,0,30,60]) {
            const result = await render(engine,renderId,frame);
            const prior = frames.get(frame);
            if (prior) expect(result).toEqual(prior);
            frames.set(frame,result);
            const png = native.get(frame);
            if (png) {
              let difference = 0;
              for (let i=0;i<png.length;i++) if (i%4!==3) difference = Math.max(difference,Math.abs(png[i]!-result.pixels[i]!));
              expect(difference,kind).toBeLessThanOrEqual(2);
            }
          }
          expect(frames.get(0)!.pixels).not.toEqual(frames.get(60)!.pixels);
          const reopened = open();
          try { expect(await render(reopened.engine,reopened.renderId,60)).toEqual(frames.get(60)!); }
          finally { reopened.engine.free(); }
          if (environment) {
            const resources = JSON.parse(engine.compiled_execution_resources_json(renderId));
            const resource = resources.find((r:any)=>r.kind==='environment-bytes');
            expect(resource).toBeDefined();
            const bytes = engine.compiled_resource_bytes(renderId,'environment-bytes',resource.contentDigest);
            bytes[bytes.length-1] ^= 1;
            expect(()=>engine.register_scene3d_environment(resource.contentDigest,bytes)).toThrow('digest');
          }
        } finally { engine.free(); }
      } finally { server.kill(); await server.exited; }
    }
  } finally { await rm(dir,{recursive:true,force:true}); }
},180_000);
