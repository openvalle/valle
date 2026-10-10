import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import type { CanvasKit, Surface } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../../../generated/web/valle_engine.js";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import { drawPyramidFilter, drawPyramidFilterCooperative } from "./pyramid.ts";

initSync({module:await readFile(new URL("../../../generated/web/valle_engine_bg.wasm",import.meta.url))});
const {default:CanvasKitInit}=await import("canvaskit-wasm") as unknown as {
  default:(options:{locateFile(file:string):string})=>Promise<CanvasKit>;
};
const ck=await CanvasKitInit({locateFile:()=>Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm",import.meta.dir)});

test("shared pyramid recipes retain offscreen energy and match the CPU F16 reference", async () => {
  const engine=new ProductEngine(),runtime=new CanvasKitBuiltinRuntime(ck);
  runtime.admit(["pyramidCoverage","pyramidThreshold","pyramidDownsample","pyramidBlur",
    "pyramidUpsample","pyramidComposite","pyramidGlowComposite"]);
  const inputWidth=63,inputHeight=45,width=80,height=58;
  const bytes=new Uint8Array(inputWidth*inputHeight*8),view=new DataView(bytes.buffer);
  for(let y=5;y<40;y++)for(let x=0;x<45;x++){
    const at=(y*inputWidth+x)*8;
    const value=x<10?[0x4000,0x4000,0x4000,0x3c00]:[0x3400,0x3000,0x3600,0x3800];
    value.forEach((half,channel)=>view.setUint16(at+channel*2,half,true));
  }
  const info={width:inputWidth,height:inputHeight,colorType:ck.ColorType.RGBA_F16,
    alphaType:ck.AlphaType.Premul,colorSpace:ck.ColorSpace.SRGB};
  const input=ck.MakeImage(info,bytes,inputWidth*8)!;
  const checked=new Set<Surface>(),owned=new Map<Surface,ReturnType<typeof ck.Malloc>>();
  const make=(w:number,h:number)=>{
    const memory=ck.Malloc(Uint8Array,w*h*8);
    const surface=ck.MakeRasterDirectSurface({...info,width:w,height:h},memory,w*8)!;
    owned.set(surface,memory);return surface;
  };
  const pool={gpu:false,transient(w:number,h:number){const surface=make(w,h);checked.add(surface);return surface;},
    releaseTransient(surface:Surface){expect(checked.delete(surface)).toBe(true);surface.delete();ck.Free(owned.get(surface)!);owned.delete(surface);}};
  const target=make(width,height);
  try {
    const color={red:0.1,green:0.8,blue:1,alpha:0.7};
    for(const [offsetX,offsetY] of [[-8,7],[15,-6]])for(const radius of [0,4,12,48])for(const kind of ["glow","bloom"] as const){
      const filter=kind==="glow"?{kind,color,intensity:0.6,radius}:{kind,threshold:0.45,knee:0.2,intensity:0.6,radius};
      const recipe=engine.plan_pyramid_filter(inputWidth,inputHeight,width,height,offsetX!,offsetY!,JSON.stringify(filter));
      target.getCanvas().clear(ck.TRANSPARENT);
      drawPyramidFilter(ck,runtime,pool,target.getCanvas(),input,
        {x:offsetX!,y:offsetY!,width:inputWidth,height:inputHeight},{x:0,y:0,width,height},kind,recipe);
      const actual=Float32Array.from(target.getCanvas().readPixels(0,0,{...info,width,height,colorType:ck.ColorType.RGBA_F32}) as Float32Array);
      const expected=kind==="glow"?engine.apply_glow_f16(bytes,inputWidth,inputHeight,width,height,offsetX!,offsetY!,JSON.stringify(color),0.6,radius)
        :engine.apply_bloom_f16(bytes,inputWidth,inputHeight,width,height,offsetX!,offsetY!,0.45,0.2,0.6,radius);
      const image=ck.MakeImage({...info,width,height},expected,width*8)!;
      try {
        const pixels=image.readPixels(0,0,{...info,width,height,colorType:ck.ColorType.RGBA_F32}) as Float32Array;
        let max=0;for(let i=0;i<width*height*4;i++)max=Math.max(max,Math.abs(actual[i]!-pixels[i]!));
        expect(max,`${kind} radius=${radius} offset=${offsetX},${offsetY}`).toBeLessThanOrEqual(0.003);
      } finally { image.delete(); }
      expect(checked.size).toBe(0);
      if (radius === 48 && kind === "bloom" && offsetX === 15) {
        target.getCanvas().clear(ck.TRANSPARENT);
        await drawPyramidFilterCooperative(() => Bun.sleep(0), ck,runtime,pool,target.getCanvas(),input,
          {x:offsetX!,y:offsetY!,width:inputWidth,height:inputHeight},{x:0,y:0,width,height},kind,recipe);
        const cooperative=Float32Array.from(target.getCanvas().readPixels(0,0,
          {...info,width,height,colorType:ck.ColorType.RGBA_F32}) as Float32Array);
        expect(cooperative.subarray(0,width*height*4)).toEqual(actual.subarray(0,width*height*4));
        expect(checked.size).toBe(0);
        const cancelled=new Error("preview cancelled");
        await expect(drawPyramidFilterCooperative(async()=>{throw cancelled;}, ck,runtime,pool,
          target.getCanvas(),input,{x:offsetX!,y:offsetY!,width:inputWidth,height:inputHeight},
          {x:0,y:0,width,height},kind,recipe)).rejects.toThrow("preview cancelled");
        expect(checked.size).toBe(0);
      }
    }
    expect(()=>engine.plan_pyramid_filter(63,45,80,58,0,0,JSON.stringify({kind:"blur",sigmaX:1,sigmaY:1}))).toThrow();
  } finally {
    input.delete();target.delete();ck.Free(owned.get(target)!);owned.delete(target);
    for(const [surface,memory] of owned){surface.delete();ck.Free(memory);}
    runtime.dispose();engine.free();
  }
}, 30_000);
