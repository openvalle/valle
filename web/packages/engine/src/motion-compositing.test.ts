import { expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { CanvasKit } from "canvaskit-wasm";
import { initSync, ProductEngine } from "../generated/web/valle_engine.js";
import { decodePackedAbi, RESOURCE_REQUESTS_ABI, type PackedValue } from "./abi/packed.ts";
import { CanvasKitExecutor, type CanvasKitExternalObject } from "./executor/canvaskit/executor.ts";

// Build the CLI and WASM first. Each package comes from actual Motion authoring;
// analytic probes keep agreement between two equally wrong backends from passing.
const root = resolve(import.meta.dir, "../../../../");
const cli = process.env.VALLE_TEST_CLI ?? join(root, "target/debug/valle");
const backend = process.env.VALLE_TEST_NATIVE_BACKEND ?? "raster";
const { default: CanvasKitInit } = await import("canvaskit-wasm") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
type Probe = readonly [number, number, number, number, number, number?];
type Case = { name: string; body: string; probes: readonly Probe[]; frameProbes?: Record<number,readonly Probe[]>; frames?: number[]; source?: string; width?: number; height?: number; crossParityTolerance?: number; atlasAsset?: boolean };
const view = (color: string, extra = "") => `<View style={{position:"absolute",left:0,top:0,width:64,height:64,backgroundColor:"${color}"${extra}}}/>`;
const circle = (fill = '"#fff"') => `<Circle cx={32} cy={32} r={16} fill={${fill}}/>`;
const mask = (source: string, attrs = "", sourceAttrs = "") => `<Mask key="mask" rect={rect(0,0,64,64)} ${attrs} style={{width:64,height:64}}>
  <MaskSource key="source" ${sourceAttrs}>${source}</MaskSource>${view("#fff")}</Mask>`;

test("Motion composition, easing and color interpolation match analytic pixels on Native and CanvasKit", async () => {
  const ck = await CanvasKitInit({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir) });
  initSync({ module: await readFile(new URL("../generated/web/valle_engine_bg.wasm", import.meta.url)) });
  const dir = await mkdtemp(join(tmpdir(), "valle-compositing-"));
  const atlasBytes = await readFile(join(root, "crates/valle-compiler/tests/fixtures/motion/composition/atlas-sprites.png"));
  const atlasSource = await readFile(join(root, "crates/valle-compiler/tests/fixtures/motion/composition/atlas-geometry-batch.motion.tsx"), "utf8");
  await writeFile(join(dir, "atlas-sprites.png"), atlasBytes);
  const spawn = (args: string[]) => Bun.spawn([cli, "--json", ...args], {
    cwd:dir, env:{ ...process.env, VALLE_HOME:join(dir,"home") }, stdout:"pipe", stderr:"pipe",
  });
  const cases: Case[] = [
    { name:"regression-captured-map", source:`export const composition={width:64,height:64,fps:30,duration:2};
      const rows=[{id:"a",x:0},{id:"b",x:16},{id:"c",x:32}];
      export default function Main(ctx){const k=ctx.progress*2;return <Scene style={{width:64,height:64,backgroundColor:"black"}}>
        {rows.map(row=><View key={row.id} className="absolute" style={{left:row.x+k*10,top:20,width:8,height:8,backgroundColor:"white"}}/>)}</Scene>}`, body:"", probes:[],
      frameProbes:{0:[[1,24,255,255,255],[9,24,0,0,0]],30:[[11,24,255,255,255],[1,24,0,0,0]]} },
    { name:"regression-chromatic-alpha", source:`export const composition={width:64,height:64,fps:30,duration:2};
      export default function Main(ctx){return <Scene style={{width:64,height:64}}><View style={{width:64,height:64,
        backgroundColor:"#ffffff80",filter:\`chromatic-aberration(\${ctx.seconds}px)\`}}/></Scene>}`, body:"",
      probes:[[32,32,255,255,255,128]], frames:[0,30,0] },
    { name:"regression-grain-black-white", body:`<View style={{width:64,height:64,filter:"film-grain(7 0.5 1px)"}}>
      ${view("black",",width:32")}${view("white",",left:32,width:32")}</View>`, probes:[[16,32,0,0,0],[48,32,255,255,255]] },
    { name:"batch-rotation", body:`<GeometryBatch geometry="rect" positions={[point(8,16),point(40,16)]}
      sizes={[point(16,8)]} fills="#ffd000" rotations={[0,36]}
      style={{position:"absolute",left:0,top:0,width:64,height:64}}/>`, probes:[[16,20,255,208,0],[48,20,255,208,0]] },
    { name:"batch-skew-stroke", body:`<GeometryBatch geometry="rect" positions={[point(8,16),point(40,16)]}
      sizes={[point(16,8)]} fills="#ffd000"
      rotations={field({from:[0,0],to:[0,12],progress:ctx.progress})}
      skewXs={field({from:[0,0],to:[0,24],progress:ctx.progress})}
      strokeWidths={field({from:[0,0],to:[0,0.1],progress:ctx.progress})}
      style={{position:"absolute",left:0,top:0,width:64,height:64}}/>`, probes:[[16,20,255,208,0],[48,20,255,208,0]] },
    { name:"geometry-batch-atlas", source:atlasSource,
      body:"", width:320, height:160, frames:[0,30,0], atlasAsset:true,
      probes:[[64,60,255,255,255],[64,100,128,128,128],
        [224,60,109,164,218],[224,100,52,80,109],[8,8,16,16,16]], crossParityTolerance:3 },
    { name:"geometry-batch-atlas-opaque", source:atlasSource.replace('fills={["#ffffff", "#80c0ff"]}', 'fills="#ffffff"').replace("opacities={[1, 0.7]}", "opacities={[1, 1]}"),
      body:"", width:320, height:160, frames:[0,30,0], atlasAsset:true,
      probes:[[64,60,255,255,255],[64,100,128,128,128],[224,60,255,255,255],[224,100,128,128,128]], crossParityTolerance:3 },
    { name:"geometry-batch-atlas-affine", source:atlasSource.replace("sizes={[point(80, 80), point(80, 80)]}", "sizes={[point(80, 40), point(80, 80)]}")
        .replace("rotations={[0, 36]}", "rotations={[0, 36]} skewXs={[0, 12]}"),
      body:"", width:320, height:160, frames:[0,30,0], atlasAsset:true,
      probes:[[64,50,255,255,255],[64,70,128,128,128],[8,8,16,16,16]], crossParityTolerance:3 },
    { name:"geometry-batch-atlas-fractional", source:atlasSource.replace("rect(0, 0, 0.5, 1)", "rect(0.00390625, 0, 0.4921875, 1)"),
      body:"", width:320, height:160, frames:[0,30,0], atlasAsset:true,
      probes:[[64,60,255,255,255],[64,100,128,128,128],[8,8,16,16,16]], crossParityTolerance:3 },
    { name:"geometry-batch-atlas-right", source:atlasSource.replace("rect(0, 0, 0.5, 1)", "rect(0.5, 0, 0.5, 1)")
        .replace('fills={["#ffffff", "#80c0ff"]}', 'fills="#ffffff"').replace("opacities={[1, 0.7]}", "opacities={[1, 1]}"),
      body:"", width:320, height:160, frames:[0,30,0], atlasAsset:true,
      probes:[[64,60,255,0,255],[64,100,255,0,255],[224,60,255,0,255]], crossParityTolerance:3 },
    { name:"plus-linear", body:view("#800000")+view("#800000", ',mixBlendMode:"plus-lighter",width:32'), probes:[[16,32,175,0,0],[48,32,128,0,0]] },
    { name:"plus-opacity", body:view("#800000")+view("#800000", ',mixBlendMode:"plus-lighter",opacity:0.5,width:32'), probes:[[16,32,154,0,0],[48,32,128,0,0]] },
    { name:"plus-transparent-group", body:`<Group style={{position:"relative",width:64,height:64,opacity:0.5,isolation:"isolate"}}>${view("#800000",',opacity:0.5')}${view("#800000",',opacity:0.5,mixBlendMode:"plus-lighter"')}</Group>`, probes:[[32,32,92,0,0]] },
    { name:"offscreen-blend", body:view("#800000",',mixBlendMode:"plus-lighter",opacity:0.5,left:100'), probes:[[32,32,0,0,0]] },
    { name:"screen-srgb", body:view("#800000")+view("#800000", ',mixBlendMode:"screen"'), probes:[[32,32,192,0,0]] },
    { name:"screen-explicit-srgb", body:view("#800000")+view("#800000", ',mixBlendMode:"screen",mixBlendSpace:"srgb"'), probes:[[32,32,192,0,0]] },
    { name:"screen-linear", body:view("#800000")+view("#800000", ',mixBlendMode:"screen",mixBlendSpace:"linear",width:32'), probes:[[16,32,167,0,0],[48,32,128,0,0]] },
    { name:"screen-linear-opacity", body:view("#800000")+view("#800000", ',mixBlendMode:"screen",mixBlendSpace:"linear",opacity:0.5'), probes:[[32,32,149,0,0]] },
    { name:"screen-linear-transparent", body:`<Group style={{position:"relative",width:64,height:64,opacity:0.5,isolation:"isolate"}}>${view("#800000",',opacity:0.5')}${view("#800000",',opacity:0.5,mixBlendMode:"screen",mixBlendSpace:"linear"')}</Group>`, probes:[[32,32,90,0,0]] },
    { name:"screen-linear-mask", body:view("#800000")+`<Mask rect={rect(0,0,64,64)} style={{width:64,height:64,mixBlendMode:"screen",mixBlendSpace:"linear"}}><MaskSource>${circle()}</MaskSource>${view("#800000")}</Mask>`, probes:[[32,32,167,0,0],[2,2,128,0,0]] },
    { name:"screen-linear-gradient", body:`<View style={{position:"absolute",width:64,height:64,backgroundImage:"linear-gradient(90deg in srgb, #206080, #803010)"}}/>`+view("#80a040", ',mixBlendMode:"screen",mixBlendSpace:"linear",opacity:0.5,transform:"rotate(10deg)"'), probes:[] },
    { name:"blend-space-dynamic", body:view("#800000")+view("#800000", ',mixBlendMode:"screen",mixBlendSpace:ctx.seconds<0.5?"linear":"srgb"'), probes:[], frameProbes:{0:[[32,32,167,0,0]],30:[[32,32,192,0,0]]} },
    { name:"blend-space-not-inherited", body:`<Group style={{mixBlendSpace:"linear",width:64,height:64}}>${view("#800000")}${view("#800000", ',mixBlendMode:"screen"')}</Group>`, probes:[[32,32,192,0,0]] },
    { name:"multiply-linear", body:view("#800000")+view("#800000", ',mixBlendMode:"multiply",mixBlendSpace:"linear"'), probes:[[32,32,61,0,0]] },
    { name:"plus-space-independent", body:view("#800000")+view("#800000", ',mixBlendMode:"plus-lighter",mixBlendSpace:"linear",width:32'), probes:[[16,32,175,0,0],[48,32,128,0,0]] },
    { name:"alpha", body:mask(circle()), probes:[[32,32,255,255,255],[2,2,0,0,0]] },
    { name:"alpha-half", body:mask(circle(), 'mode="alpha"', 'style={{opacity:0.5}}'), probes:[[32,32,188,188,188],[2,2,0,0,0]] },
    { name:"alpha-inverse", body:mask(circle(), 'invert'), probes:[[32,32,0,0,0],[2,2,255,255,255]] },
    { name:"luminance-red", body:mask(circle('"#800000"'), 'mode="luminance"'), probes:[[32,32,60,60,60],[2,2,0,0,0]] },
    { name:"luminance-half", body:mask(circle(), 'mode="luminance"', 'style={{opacity:0.5}}'), probes:[[32,32,188,188,188],[2,2,0,0,0]] },
    { name:"luminance-inverse", body:mask(circle('"#800000"'), 'mode="luminance" invert={true}'), probes:[[32,32,250,250,250],[2,2,255,255,255]] },
    { name:"empty", body:mask(""), probes:[[32,32,0,0,0],[2,2,0,0,0]] },
    { name:"hidden", body:mask(circle(), '', 'style={{display:"none"}}'), probes:[[32,32,0,0,0]] },
    { name:"inactive", body:mask(circle(), '', 'visible={false}'), probes:[[32,32,0,0,0]] },
    { name:"empty-inverse", body:mask("", 'invert'), probes:[[32,32,255,255,255],[2,2,255,255,255]] },
    { name:"source-transform", body:mask(circle(), '', 'style={{translate:point(16,0)}}'), probes:[[48,32,255,255,255],[16,32,0,0,0]] },
    { name:"nested", body:mask(mask(circle(), 'mode="luminance"').replace('key="mask"','key="inner"').replace('key="source"','key="inner-source"')), probes:[[32,32,255,255,255],[2,2,0,0,0]] },
    { name:"gradient", body:mask(circle('radialGradient(point(32,32),16,[gradientStop(0,"#fff"),gradientStop(1,"#ffffff00")])')), probes:[[2,2,0,0,0]] },
    // Circle lowers to this full arc too: compare mask/clip coverage with identical geometry.
    { name:"clip", body:`<Clip path={arc(point(32,32),16,0,TAU)} style={{width:64,height:64}}>${view("#fff")}</Clip>`, probes:[[32,32,255,255,255],[2,2,0,0,0]] },
  ];
  const images = new Map<string, Uint8Array>();
  const frameImages = new Map<string, { web: Uint8Array; native: Uint8Array }>();
  const easingPositions: Array<{frame:number; back:number; steps:number}> = [];
  const fixtureDir = join(root,"crates/valle-compiler/tests/fixtures/motion/composition");
  const iris = await readFile(join(fixtureDir,"subtree-mask.motion.tsx"),"utf8");
  const instances = await readFile(join(fixtureDir,"grid-instances.motion.tsx"),"utf8");
  const instancesCircle = await readFile(join(fixtureDir,"circle-instances.motion.tsx"),"utf8");
  const instancesPath = await readFile(join(fixtureDir,"path-instances.motion.tsx"),"utf8");
  const instancesPathOpacity = instancesPath.replace('translate: point(dot.x + ctx.seconds * 3, dot.y)',
    'translate: point(dot.x + ctx.seconds * 3, dot.y), opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1');
  const instancesPathStroke = instancesPathOpacity.replace('fill={dot.color}',
    'fill={dot.color} stroke={dot.color} strokeWidth={1.5}');
  const instancesPathStrokeDynamic = instancesPathStroke.replace('strokeWidth={1.5}',
    'strokeWidth={0.25 + dot.delay + ctx.progress}');
  const instancesPathStrokeColors = instancesPathStroke.replace('stroke={dot.color}',
    "stroke={dot.delay < ctx.progress ? '#ff0000' : '#00ff00'}");
  const instancesPathStrokeColorsDynamic = instancesPathStrokeDynamic.replace('stroke={dot.color}',
    "stroke={dot.delay < ctx.progress ? '#ff0000' : '#00ff00'}");
  const instancesPathStrokeOnly = instancesPathStrokeColors.replace('fill={dot.color}', 'fill="#00000000"');
  const instancesPathStrokeStyle = instancesPath.replace('fill={dot.color}',
    'fill="none" stroke={dot.color} strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="bevel" strokeMiterlimit="6" strokeDasharray="3 2" strokeDashoffset={dot.delay * 5 + ctx.progress}');
  const instancesPathStrokeMiter = instancesPathStrokeStyle.replace('strokeLinecap="round" strokeLinejoin="bevel" strokeMiterlimit="6"',
    'strokeLinecap="square" strokeLinejoin="miter" strokeMiterlimit="8"').replace('strokeDashoffset={dot.delay * 5 + ctx.progress}', 'strokeDashoffset={1.25}');
  const instancesPathStrokeStyleColors = instancesPath.replace('fill={dot.color}',
    'fill={dot.color} stroke={dot.delay < ctx.progress ? "#ff0000" : "#00ff00"} strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="bevel" strokeDasharray="3 2" strokeDashoffset={dot.delay * 5 + ctx.progress}');
  const instancesPathRotation = instancesPath.replace('translate: point(dot.x + ctx.seconds * 3, dot.y)',
    'translate: point(dot.x + ctx.seconds * 3, dot.y), rotate: `${dot.delay * 180 + ctx.seconds * 30}deg`, transformOrigin: point(0, 0), opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1');
  const instancesPathScale = instancesPath.replace('translate: point(dot.x + ctx.seconds * 3, dot.y)',
    'translate: point(dot.x + ctx.seconds * 3, dot.y), scale: point(0.6 + dot.delay * 0.4, 0.7 + ctx.progress * 0.2), transformOrigin: point(0, 0)');
  const instancesPathScaleMirror = instancesPathScale.replace('0.6 + dot.delay * 0.4', '0.6 - dot.delay');
  const instancesPathSkew = instancesPath.replace('translate: point(dot.x + ctx.seconds * 3, dot.y)',
    'translate: point(dot.x + ctx.seconds * 3, dot.y), transform: `skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`, transformOrigin: point(0, 0)');
  const instancesPathScaleRotation = instancesPath.replace('translate: point(dot.x + ctx.seconds * 3, dot.y)',
    'translate: point(dot.x + ctx.seconds * 3, dot.y), scale: point(0.6 + dot.delay * 0.4, 0.7 + ctx.progress * 0.2), rotate: `${dot.delay * 180 + ctx.seconds * 30}deg`, transformOrigin: point(0, 0), opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1');
  const instancesPathScaleRotationSkew = instancesPathScaleRotation.replace('transformOrigin: point(0, 0)',
    'transform: `skewX(${dot.delay * 40 + ctx.seconds * 10 - 20}deg)`, transformOrigin: point(0, 0)');
  const instancesPathStrokeAffine = instancesPathScaleRotationSkew.replace('fill={dot.color}',
    'fill={dot.color} stroke={dot.color} strokeWidth={1.5}');
  const instancesPathStrokeAffineOpaque = instancesPathStrokeAffine.replace(
    ', opacity: 0.4 + dot.delay * 0.4 + ctx.progress * 0.1', '');
  const instancesPathStrokeCurve = instancesPathStrokeAffine.replace(
    'M 0 -8 L 2 -2 L 8 0 L 2 2 L 0 8 L -2 2 L -8 0 L -2 -2 Z',
    'M 0.1 -8.2 C 4.3 -8.6 8.4 -3.5 8.1 0.2 C 8.3 4.1 3.1 8.2 0 8.4 C -4 8.2 -8.1 3.7 -8.2 0 C -8.1 -3.4 -4.3 -8.5 0.1 -8.2 Z');
  const instancesFlow = await readFile(join(fixtureDir,"flow-instances.motion.tsx"),"utf8");
  const instancesCards = await readFile(join(fixtureDir,"repeated-cards.motion.tsx"),"utf8");
  const instancesCardComponent = await readFile(join(fixtureDir,"repeated-card-component.motion.tsx"),"utf8");
  const instancesGroup = instancesCards.replace('<View key={card.id}', '<Group key={card.id}')
    .replace('    </View>)}', '    </Group>)}');
  const instancesGroupComponent = instancesCardComponent.replace('return <View key="body"', 'return <Group key="body"')
    .replace('\n  </View>;', '\n  </Group>;');
  const instancesTextRoot = await readFile(join(fixtureDir,"text-root-instances.motion.tsx"),"utf8");
  const instancesTextRootDirect = instancesTextRoot.replace('<Label key={row.id} item={row} />',
    '<Text key={row.id} style={{ fontSize: 14, color: row.color }}>{row.label}</Text>');
  const instancesCardsClasses = instancesCards
    .replace('<View key={card.id} style=',
      '<View key={card.id} className={card.height > 36 ? "flex flex-col rounded-lg opacity-50" : "flex flex-col rounded-lg"} style=')
    .replace('<Group style={{ width: 106, height: 16 }}>', '<Group className="relative" style={{ width: 106, height: 16 }}>')
    .replace('<Text visible={ctx.localFrame < card.height - 6}',
      '<Text className={card.height > 36 ? "font-bold" : "font-normal"} visible={ctx.localFrame < card.height - 6}');
  const instancesCircleVaried = instancesCircle
    .replace('20.3 + (i % COLS) * (600 / COLS)', '8.13 + ((i * 73) % 600) + (i % 7) * 0.07')
    .replace('20.6 + Math.floor(i / COLS) * (320 / COLS)', '8.17 + ((i * 47) % 340) + (i % 5) * 0.09')
    .replace('3.5 + (i % 3) * 0.25', '1.37 + (i % 23) * 1.11');
  const instancesShutter = await readFile(join(fixtureDir,"shutter-instances.motion.tsx"),"utf8");
  const lazyEvaluationMixed = await readFile(join(fixtureDir,"mixed-lazy-evaluation.motion.tsx"),"utf8");
  const lazyEvaluationNested = await readFile(join(fixtureDir,"nested-lazy-evaluation.motion.tsx"),"utf8");
  const lazyEvaluationCertifiedHidden = lazyEvaluationNested.replace("ctx.seconds * 100 >= dot.i","ctx.seconds * 100 >= dot.i + 1000");
  const lazyEvaluation = await readFile(join(fixtureDir,"lazy-evaluation.motion.tsx"),"utf8");
  const lazyEvaluationAffine = await readFile(join(fixtureDir,"affine-time-dependency.motion.tsx"),"utf8");
  const lazyEvaluationAffineEager = lazyEvaluationAffine.replace("dot.i <= (ctx.seconds - 0.2) * 1000",
    "(dot.i <= (ctx.seconds - 0.2) * 1000 ? true : false)");
  const lazyEvaluationComposed = await readFile(join(fixtureDir,"mask-sibling-lazy-evaluation.motion.tsx"),"utf8");
  const particleForces = await readFile(join(fixtureDir,"particle-forces.motion.tsx"),"utf8");
  const particleForcesForces = "forces: [curlNoise({ seed: 3, scale: 0.012, strength: 220 }), drag(0.8)],";
  const particleForcesVariant = (forces:string) => particleForces.replace(particleForcesForces,forces);
  const distanceStagger = await readFile(join(fixtureDir,"distance-stagger-batch.motion.tsx"),"utf8");
  const autoMotionBlur = await readFile(join(fixtureDir,"auto-motion-blur.motion.tsx"),"utf8");
  const shutterSampling = await readFile(join(fixtureDir,"shutter-sampling.motion.tsx"),"utf8");
  const echoTrails = await readFile(join(fixtureDir,"echo-trails.motion.tsx"),"utf8");
  const timeScope = await readFile(join(fixtureDir,"time-scope.motion.tsx"),"utf8");
  const springSimulation = await readFile(join(fixtureDir,"spring-simulation.motion.tsx"),"utf8");
  const textOutline = await readFile(join(fixtureDir,"text-outline-drawing.motion.tsx"),"utf8");
  const rangeSelector = await readFile(join(fixtureDir,"text-range-selector.motion.tsx"),"utf8");
  const richTextUnits = await readFile(join(fixtureDir,"rich-text-units.motion.tsx"),"utf8");
  const multilinePathText = await readFile(join(fixtureDir,"multiline-path-text.motion.tsx"),"utf8");
  const nodeGlow = await readFile(join(fixtureDir,"node-glow.motion.tsx"),"utf8");
  const sceneBloom = await readFile(join(fixtureDir,"scene-bloom.motion.tsx"),"utf8");
  const chromaticAberration = await readFile(join(fixtureDir,"chromatic-aberration.motion.tsx"),"utf8");
  const radialBlur = await readFile(join(fixtureDir,"radial-blur.motion.tsx"),"utf8");
  const filmGrain = await readFile(join(fixtureDir,"film-grain.motion.tsx"),"utf8");
  const lensDistortion = await readFile(join(fixtureDir,"lens-distortion.motion.tsx"),"utf8");
  const lensDistortionCrop = lensDistortion.replace("width: 640, height: 360", "width: 320, height: 360")
    .replace("left: 200, top: 100", "left: 300, top: 100")
    .replace("left: 52, top: 20", "left: 30, top: 20");
  const starToCircleMorph = await readFile(join(fixtureDir,"star-to-circle-morph.motion.tsx"),"utf8");
  const pathMorphSequence = await readFile(join(fixtureDir,"path-morph-sequence.motion.tsx"),"utf8");
  const pathMorphSequenceControl = pathMorphSequence.replace("morphSequence([STAR, SQUARE, CIRCLE], [0, 0.5, 1], ctx.progress)","SQUARE");
  const convexMorph = await readFile(join(fixtureDir,"convex-morph.motion.tsx"),"utf8");
  const convexControl = convexMorph.replace('morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "convex" })',"SB");
  const arcLengthMorph = await readFile(join(fixtureDir,"arc-length-morph.motion.tsx"),"utf8");
  const arcLengthControl = arcLengthMorph.replace('morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "arcLength" })',"resamplePath(SB, 256)");
  const compatibleMorph = await readFile(join(fixtureDir,"compatible-morph.motion.tsx"),"utf8");
  const compatibleControl = compatibleMorph.replace('morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "compatible" })',"SB");
  const anchorMorph = await readFile(join(fixtureDir,"anchor-morph.motion.tsx"),"utf8");
  const anchorRotation = await readFile(join(fixtureDir,"anchor-rotation.motion.tsx"),"utf8");
  const pairedContours = await readFile(join(fixtureDir,"paired-contours.motion.tsx"),"utf8");
  const pairedAnchors = await readFile(join(fixtureDir,"paired-anchors.motion.tsx"),"utf8");
  const siblingHoleBirth = await readFile(join(fixtureDir,"sibling-hole-birth.motion.tsx"),"utf8");
  const withoutPairs = (source:string)=>source
    .replace("pairs: [[0, 1], [1, 0]],", "")
    .replace("pairs: [[0, 0, 0], [null, 1, null]],", "");
  const autoContours = withoutPairs(pairedContours);
  const autoAnchors = withoutPairs(pairedAnchors);
  const allowedIntersection = await readFile(join(fixtureDir,"allow-self-intersection.motion.tsx"),"utf8");
  const pathModifiers = await readFile(join(fixtureDir,"path-modifiers.motion.tsx"),"utf8");
  const pathModifiersExtra = await readFile(join(fixtureDir,"extended-path-modifiers.motion.tsx"),"utf8");
  const reversePathTrail = await readFile(join(fixtureDir,"reverse-path-trail.motion.tsx"),"utf8");
  const reversePathTrailForward = reversePathTrail.replace("reversePath(ORBIT)","ORBIT");
  const colorInterpolationInstances = await readFile(join(fixtureDir,"float-color-instances.motion.tsx"),"utf8");
  const richTextUnitsUniform = richTextUnits.replace("ctx.unit.index * 0.8 +", "0 +");
  const rangeSelectorNoBlur = rangeSelector.replace('blur: 8 * (1 - rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" }))','blur: 0');
  const rangeSelectorFull = rangeSelectorNoBlur.replace('opacity: rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" })','opacity: 1');
  const textOutlineControl = `export const composition={width:640,height:360,fps:30,duration:2};
    export default function Control(){return <Scene className="relative h-full w-full" style={{backgroundColor:"#101010"}}>
      <Text className="absolute" style={{left:60,top:80,fontSize:150,fontWeight:800,color:"transparent",
        WebkitTextStrokeWidth:3,WebkitTextStrokeColor:"#ffffff"}}>VALLE</Text></Scene>;}`;
  const textOutlineFilledText = textOutlineControl.replace('color:"transparent",\n        WebkitTextStrokeWidth:3,WebkitTextStrokeColor:"#ffffff"','color:"#ffffff"');
  const textOutlineFilledPath = textOutline.replace('fill="none" stroke="#ffffff" strokeWidth={3} trimEnd={ctx.progress}', 'fill="#ffffff"');
  const defaultFont = new Uint8Array(await readFile(join(root,"assets/fonts/noto/NotoSans-Variable.ttf")));
  cases.push(
    { name:"instances", source:instances, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-expanded", source:instances.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-circle", source:instancesCircle, body:"", width:640, height:360, frames:[0,15,30,15], probes:[] },
    { name:"instances-circle-expanded", source:instancesCircle.replaceAll('<Circle ','<circle '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path", source:instancesPath, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-expanded", source:instancesPath.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-opacity", source:instancesPathOpacity, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-opacity-expanded", source:instancesPathOpacity.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke", source:instancesPathStroke, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-expanded", source:instancesPathStroke.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-dynamic", source:instancesPathStrokeDynamic, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-dynamic-expanded", source:instancesPathStrokeDynamic.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-colors", source:instancesPathStrokeColors, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-colors-expanded", source:instancesPathStrokeColors.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-colors-dynamic", source:instancesPathStrokeColorsDynamic, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-colors-dynamic-expanded", source:instancesPathStrokeColorsDynamic.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-only", source:instancesPathStrokeOnly, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-only-expanded", source:instancesPathStrokeOnly.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-style", source:instancesPathStrokeStyle, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-style-expanded", source:instancesPathStrokeStyle.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-miter", source:instancesPathStrokeMiter, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-miter-expanded", source:instancesPathStrokeMiter.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-stroke-style-colors", source:instancesPathStrokeStyleColors, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-stroke-style-colors-expanded", source:instancesPathStrokeStyleColors.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-rotation", source:instancesPathRotation, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-rotation-expanded", source:instancesPathRotation.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-scale", source:instancesPathScale, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-scale-expanded", source:instancesPathScale.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-scale-mirror", source:instancesPathScaleMirror, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-scale-mirror-expanded", source:instancesPathScaleMirror.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-skew", source:instancesPathSkew, body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    { name:"instances-path-skew-expanded", source:instancesPathSkew.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[] },
    // Combined CSS rotation, scaling and opacity has one edge pixel with a 3-level
    // Native/CanvasKit coverage difference in both the template and expanded paths.
    { name:"instances-path-scale-rotation", source:instancesPathScaleRotation, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    { name:"instances-path-scale-rotation-expanded", source:instancesPathScaleRotation.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:3 },
    // The same antialiased edge differs by six levels across backends in both render paths.
    { name:"instances-path-scale-rotation-skew", source:instancesPathScaleRotationSkew, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-scale-rotation-skew-expanded", source:instancesPathScaleRotationSkew.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-affine", source:instancesPathStrokeAffine, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-affine-expanded", source:instancesPathStrokeAffine.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-affine-opaque", source:instancesPathStrokeAffineOpaque, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-affine-opaque-expanded", source:instancesPathStrokeAffineOpaque.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-curve", source:instancesPathStrokeCurve, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-path-stroke-curve-expanded", source:instancesPathStrokeCurve.replaceAll('<Path ','<path '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:6 },
    { name:"instances-flow", source:instancesFlow, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-flow-expanded", source:instancesFlow.replace('key={row.id}','key={`${row.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-cards", source:instancesCards, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-cards-expanded", source:instancesCards.replace('key={card.id}','key={`${card.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-component", source:instancesCardComponent, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-component-expanded", source:instancesCardComponent.replace('key={card.id}','key={`${card.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-group", source:instancesGroup, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-group-expanded", source:instancesGroup.replace('key={card.id}','key={`${card.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-group-component", source:instancesGroupComponent, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-group-component-expanded", source:instancesGroupComponent.replace('key={card.id}','key={`${card.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-text-root", source:instancesTextRoot, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-text-root-expanded", source:instancesTextRoot.replace('key={row.id}','key={`${row.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-text-root-direct", source:instancesTextRootDirect, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-text-root-direct-expanded", source:instancesTextRootDirect.replace('key={row.id}','key={`${row.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-cards-classes", source:instancesCardsClasses, body:"", width:160, height:180, frames:[0,30], probes:[] },
    { name:"instances-cards-classes-expanded", source:instancesCardsClasses.replace('key={card.id}','key={`${card.id}`}'), body:"", width:160, height:180, frames:[0,30], probes:[] },
    // The two Skia builds differ at a few near-tangent arc edge pixels here; relative
    // template/expanded parity remains exact on Native and bounded on CanvasKit below.
    { name:"instances-circle-varied", source:instancesCircleVaried, body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:32 },
    { name:"instances-circle-varied-expanded", source:instancesCircleVaried.replaceAll('<Circle ','<circle '), body:"", width:640, height:360, frames:[0,15,30], probes:[], crossParityTolerance:32 },
    { name:"lazy-evaluation-mixed", source:lazyEvaluationMixed, body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-mixed-expanded", source:lazyEvaluationMixed.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-nested", source:lazyEvaluationNested, body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-nested-expanded", source:lazyEvaluationNested.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-certified-hidden", source:lazyEvaluationCertifiedHidden, body:"", width:640, height:360, frames:[0,30], probes:[] },
    { name:"lazy-evaluation-certified-hidden-expanded", source:lazyEvaluationCertifiedHidden.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,30], probes:[] },
    { name:"lazy-evaluation-instanced", source:lazyEvaluation, body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-expanded", source:lazyEvaluation.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-affine", source:lazyEvaluationAffine, body:"", width:640, height:360, frames:[0,8,30], probes:[] },
    { name:"lazy-evaluation-affine-eager", source:lazyEvaluationAffineEager, body:"", width:640, height:360, frames:[0,8,30], probes:[] },
    { name:"lazy-evaluation-composed", source:lazyEvaluationComposed, body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"lazy-evaluation-composed-expanded", source:lazyEvaluationComposed.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,1,30], probes:[] },
    { name:"easing-presets", source:await readFile(join(fixtureDir,"easing-presets.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[...Array.from({length:15},(_,i)=>i*3),12], probes:[] },
    { name:"oklch-color-interpolation", source:await readFile(join(fixtureDir,"oklch-color-interpolation.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[], frameProbes:{0:[[320,180,33,64,255]],30:[[320,180,0,196,159]]} },
    { name:"oklch-color-interpolation-gradient", source:await readFile(join(fixtureDir,"float-color-gradient.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[], frameProbes:{0:[[320,180,33,64,255]],30:[[320,180,0,196,159]]} },
    { name:"oklch-color-interpolation-text", source:await readFile(join(fixtureDir,"float-color-text.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"oklch-color-interpolation-text-stroke", source:await readFile(join(fixtureDir,"float-color-text-stroke.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"oklch-color-interpolation-batch", source:await readFile(join(fixtureDir,"float-color-batch.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[], frameProbes:{0:[[320,180,33,64,255]],30:[[320,180,144,136,128]]} },
    { name:"oklch-color-interpolation-instances", source:colorInterpolationInstances, body:"", width:640, height:360, frames:[0,30,59,30], probes:[], frameProbes:{0:[[40,22,33,64,255],[360,202,33,64,255]],30:[[40,22,0,196,159],[360,202,0,196,159]]} },
    { name:"oklch-color-interpolation-instances-expanded", source:colorInterpolationInstances.replace('className="absolute"','className="absolute "'), body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"oklch-color-interpolation-glass", source:await readFile(join(fixtureDir,"float-color-glass.motion.tsx"),"utf8"), body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"css-oklab", source:await readFile(join(fixtureDir,"css-oklab.motion.tsx"),"utf8"), body:"", width:640, height:360, probes:[[320,180,140,83,162]] },
    { name:"linear-light-blend", source:await readFile(join(fixtureDir,"linear-light-blend.motion.tsx"),"utf8"), body:"", width:640, height:360, probes:[[160,180,167,0,0],[480,180,128,0,0]] },
    { name:"node-glow", source:nodeGlow, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"node-glow-half", source:nodeGlow.replace("glow(24px 1.5 #22d3ee)","glow(24px 0.75 #22d3ee)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"node-glow-no-filter", source:nodeGlow.replace(', filter: "glow(24px 1.5 #22d3ee)"', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"scene-bloom", source:sceneBloom, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"scene-bloom-half", source:sceneBloom.replace("intensity: 1.2", "intensity: 0.6"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"scene-bloom-no-filter", source:sceneBloom.replace(' bloom={{ threshold: 0.8, intensity: 1.2, radius: 48 }}', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"chromatic-aberration", source:chromaticAberration, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"chromatic-aberration-no-filter", source:chromaticAberration.replace(', filter: "chromatic-aberration(6px)"', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"radial-blur", source:radialBlur, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"radial-blur-half", source:radialBlur.replace("radial-blur(40px 20px 20px)", "radial-blur(40px 20px 10px)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"radial-blur-no-filter", source:radialBlur.replace(', filter: "radial-blur(40px 20px 20px)"', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"film-grain", source:filmGrain, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"film-grain-zero", source:filmGrain.replace("film-grain(7 0.12 2px)", "film-grain(7 0 2px)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"film-grain-seed", source:filmGrain.replace("film-grain(7 0.12 2px)", "film-grain(8 0.12 2px)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"film-grain-no-filter", source:filmGrain.replace(', filter: "film-grain(7 0.12 2px)"', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"lens-distortion", source:lensDistortion, body:"", width:640, height:360, frames:[0,30,0], probes:[] },
    { name:"lens-distortion-zero", source:lensDistortion.replace("lens-distortion(-0.3 0)", "lens-distortion(0 0)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"lens-distortion-k2", source:lensDistortion.replace("lens-distortion(-0.3 0)", "lens-distortion(-0.3 0.4)"), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"lens-distortion-no-filter", source:lensDistortion.replace(', filter: "lens-distortion(-0.3 0)"', ''), body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"lens-distortion-crop", source:lensDistortionCrop, body:"", width:320, height:360, frames:[0], probes:[] },
    { name:"lens-distortion-crop-no-filter", source:lensDistortionCrop.replace(', filter: "lens-distortion(-0.3 0)"', ''), body:"", width:320, height:360, frames:[0], probes:[] },
    { name:"additive-light", source:await readFile(join(fixtureDir,"additive-light.motion.tsx"),"utf8"), body:"", width:640, height:360, probes:[[160,180,175,0,0],[480,180,128,0,0]] },
    { name:"subtree-mask", source:iris, body:"", width:640, height:360, probes:[[320,180,33,64,255],[0,0,16,16,16]], frameProbes:{ 0:[[420,180,16,16,16]], 30:[[420,180,33,64,255]] } },
    { name:"subtree-mask-soft", source:iris.replace('fill="#ffffff"','fill={radialGradient(point(320,180),160,[gradientStop(0,"#ffffff"),gradientStop(1,"#ffffff00")])}'), body:"", width:640, height:360, frames:[30], probes:[[0,0,16,16,16]] },
    { name:"geometry-batch", source:await readFile(join(fixtureDir,"path-geometry-batch.motion.tsx"),"utf8"), body:"", width:640, height:360, probes:[[160,180,255,208,0],[480,180,255,208,0]] },
    { name:"geometry-batch-distance", source:distanceStagger, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"geometry-batch-distance-zero-array", source:distanceStagger.replace("stagger: DELAYS", "stagger: [0,0,0,0,0]"), body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"geometry-batch-distance-zero-step", source:distanceStagger.replace("stagger: DELAYS", "stagger: 0"), body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"auto-motion-blur", source:autoMotionBlur, body:"", width:640, height:360, frames:[10,45,10], probes:[] },
    { name:"auto-motion-blur-no-auto", source:autoMotionBlur.replace('motionBlur: "auto"',""), body:"", width:640, height:360, frames:[10,45], probes:[] },
    { name:"shutter-sampling", source:shutterSampling, body:"", width:640, height:360, frames:[20,45,20], probes:[] },
    { name:"shutter-sampling-no-shutter", source:shutterSampling.replace('<Shutter key="shutter" samples={8} angle={180}>','<Group>').replace('</Shutter>','</Group>'), body:"", width:640, height:360, frames:[20], probes:[] },
    { name:"instances-shutter", source:instancesShutter, body:"", width:320, height:120, frames:[0,20,40,20], probes:[] },
    { name:"instances-shutter-expanded", source:instancesShutter.replace('className="absolute"','className="absolute "'), body:"", width:320, height:120, frames:[0,20,40], probes:[] },
    { name:"echo-trails", source:echoTrails, body:"", width:640, height:360, frames:[30,50,30], probes:[] },
    { name:"echo-trails-no-echo", source:echoTrails.replace('<Echo key="echo" count={4} interval={3} decay={0.5}>','<Group>').replace('</Echo>','</Group>'), body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"time-scope", source:timeScope, body:"", width:640, height:360, frames:[15,45,15], probes:[], frameProbes:{15:[[100,180,255,255,255],[130,180,16,16,16]],45:[[300,180,255,255,255],[321,180,16,16,16]]} },
    { name:"time-scope-no-scope", source:timeScope.replace('<TimeScope key="scope" offset={0.5} speed={2}>','').replace('</TimeScope>',''), body:"", width:640, height:360, frames:[15,45], probes:[] },
    { name:"time-scope-identity", source:timeScope.replace('offset={0.5} speed={2}','offset={0} speed={1}'), body:"", width:640, height:360, frames:[15,45], probes:[] },
    { name:"spring-simulation", source:springSimulation, body:"", width:640, height:360, frames:[0,15,55,15], probes:[],
      frameProbes:{0:[[110,180,255,255,255],[99,180,16,16,16]],15:[[380,180,255,255,255],[310,180,16,16,16]],55:[[310,180,255,255,255],[330,180,16,16,16]]} },
    { name:"text-outline", source:textOutline, body:"", width:640, height:360, frames:[0,6,30,54,59,30], probes:[] },
    { name:"text-outline-control", source:textOutlineControl, body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"text-outline-fill-path", source:textOutlineFilledPath, body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"text-outline-fill-text", source:textOutlineFilledText, body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"range-selector", source:rangeSelector, body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"range-selector-no-blur", source:rangeSelectorNoBlur, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"range-selector-full", source:rangeSelectorFull, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"rich-text-units", source:richTextUnits, body:"", width:640, height:360, frames:[10,20,10], probes:[] },
    { name:"rich-text-units-uniform", source:richTextUnitsUniform, body:"", width:640, height:360, frames:[10,20], probes:[] },
    { name:"multiline-path-text", source:multilinePathText, body:"", width:640, height:360, frames:[0], probes:[] },
    { name:"star-to-circle-morph", source:starToCircleMorph, body:"", width:640, height:360, frames:[0,30,59,30], probes:[] },
    { name:"path-morph-sequence", source:pathMorphSequence, body:"", width:640, height:360, frames:[15,30,45,30], probes:[] },
    { name:"path-morph-sequence-control", source:pathMorphSequenceControl, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"convex-morph", source:convexMorph, body:"", width:640, height:360, frames:[0,15,30,45,59,30], probes:[],
      frameProbes:{0:[[100,90,255,255,255],[300,90,16,16,16],[100,280,34,211,238]],
        30:[[300,90,255,255,255],[100,90,16,16,16],[490,280,34,211,238]],
        59:[[480,90,255,255,255],[300,90,16,16,16],[300,280,34,211,238]]} },
    { name:"convex-control", source:convexControl, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"arc-length-morph", source:arcLengthMorph, body:"", width:640, height:360, frames:[0,15,30,45,59,30], probes:[],
      frameProbes:{0:[[50,50,255,255,255],[90,80,16,16,16],[50,230,34,211,238]],
        30:[[200,50,255,255,255],[260,80,16,16,16],[280,230,34,211,238]],
        59:[[350,50,255,255,255],[420,80,16,16,16],[480,230,34,211,238]]} },
    { name:"arc-length-control", source:arcLengthControl, body:"", width:640, height:360, frames:[30], probes:[] },
    { name:"compatible-morph", source:compatibleMorph, body:"", width:160, height:120, frames:[0,15,30,45,59,30], probes:[],
      frameProbes:{0:[[25,20,255,255,255],[110,20,16,16,16],[25,82,34,211,238]],
        30:[[110,82,34,211,238]],
        59:[[110,20,255,255,255]]} },
    { name:"compatible-control", source:compatibleControl, body:"", width:160, height:120, frames:[30], probes:[] },
    { name:"anchor-morph", source:anchorMorph, body:"", width:640, height:360, frames:[0,15,30,45,59,30], probes:[],
      frameProbes:{0:[[50,50,255,255,255],[90,80,16,16,16],[50,230,34,211,238]],
        30:[[200,50,255,255,255],[260,80,16,16,16],[280,230,34,211,238]],
        59:[[350,50,255,255,255],[420,80,16,16,16],[480,230,34,211,238]]} },
    { name:"anchor-rotation", source:anchorRotation, body:"", width:640, height:240, frames:[0,30,59,30], probes:[],
      frameProbes:{30:[[320,60,255,255,255],[320,170,34,211,238],[360,60,16,16,16],[360,170,34,211,238]]} },
    { name:"paired-contours", source:pairedContours, body:"", width:640, height:360, frames:[0,15,30,45,59,30], probes:[],
      frameProbes:{0:[[80,70,255,255,255],[280,70,255,255,255],[460,260,34,211,238]],
        30:[[70,70,255,255,255],[270,70,255,255,255],[460,260,16,16,16],[420,220,34,211,238]],
        59:[[90,70,255,255,255],[290,70,255,255,255]]} },
    { name:"sibling-hole-birth", source:siblingHoleBirth, body:"", width:140, height:140, frames:[0,15,30,59,15], probes:[],
      frameProbes:{0:[[25,70,255,255,255],[25,103,255,255,255],[70,70,16,16,16],[115,70,255,255,255]],
        15:[[25,70,255,255,255],[25,103,16,16,16],[70,70,16,16,16],[115,70,255,255,255]],
        30:[[25,70,255,255,255],[25,92,16,16,16],[70,70,16,16,16],[115,70,255,255,255]],
        59:[[25,70,16,16,16],[25,92,255,255,255],[70,70,16,16,16],[115,70,255,255,255]]} },
    { name:"paired-anchors", source:pairedAnchors, body:"", width:640, height:360, frames:[0,30,59,30], probes:[],
      frameProbes:{0:[[80,70,255,255,255],[280,70,255,255,255],[460,260,34,211,238]],
        30:[[70,70,255,255,255],[270,70,255,255,255],[460,260,16,16,16],[420,220,34,211,238]],
        59:[[90,70,255,255,255],[290,70,255,255,255]]} },
    { name:"auto-contours", source:autoContours, body:"", width:640, height:360, frames:[0,30,59,30], probes:[],
      frameProbes:{30:[[70,70,255,255,255],[270,70,255,255,255],[460,260,16,16,16]]} },
    { name:"auto-anchors", source:autoAnchors, body:"", width:640, height:360, frames:[0,30,59,30], probes:[],
      frameProbes:{30:[[70,70,255,255,255],[270,70,255,255,255],[460,260,16,16,16]]} },
    { name:"allow-self-intersection", source:allowedIntersection, body:"", width:64, height:64, frames:[0,27,59,27], probes:[] },
    { name:"path-modifiers", source:pathModifiers, body:"", width:640, height:360, frames:[10,40,10],
      probes:[[40,40,16,16,16],[80,80,139,123,255],[306,90,34,211,238],[319,70,34,211,238],[470,250,244,114,182]] },
    { name:"path-modifiers-extra", source:pathModifiersExtra, body:"", width:640, height:360, frames:[0,30,59,0],
      probes:[[140,90,244,114,182],[460,90,139,123,255],[320,190,16,16,16],[320,155,163,230,53]],
      frameProbes:{0:[[470,240,16,16,16]],59:[[470,250,255,208,0]]} },
    { name:"reverse-path-trail", source:reversePathTrail, body:"", width:640, height:360, frames:[0,15,0], probes:[],
      frameProbes:{0:[[460,180,255,255,255]],15:[[363,47,255,255,255],[363,313,16,16,16],[410,70,157,157,157]]} },
    { name:"reverse-path-trail-forward", source:reversePathTrailForward, body:"", width:640, height:360, frames:[15], probes:[],
      frameProbes:{15:[[363,313,255,255,255],[363,47,16,16,16],[410,290,157,157,157]]} },
    { name:"particle-forces", source:particleForces, body:"", width:640, height:360, frames:[45,0,45], probes:[] },
    { name:"particle-forces-none", source:particleForcesVariant(""), body:"", width:640, height:360, frames:[45], probes:[] },
    { name:"particle-forces-zero-curl", source:particleForcesVariant("forces: [curlNoise({ seed: 3, scale: 0.012, strength: 0 }), drag(0.8)],"), body:"", width:640, height:360, frames:[45], probes:[] },
    { name:"particle-forces-drag-only", source:particleForcesVariant("forces: [drag(0.8)],"), body:"", width:640, height:360, frames:[45], probes:[] },
    { name:"particle-forces-zero-drag", source:particleForcesVariant("forces: [curlNoise({ seed: 3, scale: 0.012, strength: 220 }), drag(0)],"), body:"", width:640, height:360, frames:[45], probes:[] },
    { name:"particle-forces-curl-only", source:particleForcesVariant("forces: [curlNoise({ seed: 3, scale: 0.012, strength: 220 })],"), body:"", width:640, height:360, frames:[45], probes:[] },
  );
  const gradient = (value:string) => `<View style={{width:64,height:64,backgroundImage:${JSON.stringify(value)}}}/>`;
  cases.push(
    { name:"gradient-srgb", body:gradient("linear-gradient(90deg in srgb, red, blue)"), probes:[[32,32,126,0,129]] },
    { name:"gradient-linear", body:gradient("linear-gradient(90deg in srgb-linear, red, blue)"), probes:[[32,32,186,0,189]] },
    { name:"gradient-oklab", body:gradient("linear-gradient(90deg in oklab, red, blue)"), probes:[[32,32,139,83,163]] },
    { name:"gradient-default", body:gradient("linear-gradient(90deg, red, blue)"), probes:[[32,32,139,83,163]] },
    { name:"gradient-dynamic", body:`<View style={{width:64,height:64,backgroundImage:ctx.seconds<0.5?"linear-gradient(90deg in srgb, red, blue)":"linear-gradient(90deg in oklab, red, blue)"}}/>`, probes:[], frameProbes:{0:[[32,32,126,0,129]],30:[[32,32,139,83,163]]} },
    { name:"gradient-hard", body:gradient("linear-gradient(90deg in oklab, red 0% 50%, blue 50% 100%)"), probes:[[31,32,255,0,0],[32,32,0,0,255]] },
    { name:"gradient-alpha", body:gradient("linear-gradient(90deg in oklab, #ff000000, blue)"), probes:[[32,32,0,0,189]] },
    { name:"gradient-radial", body:gradient("radial-gradient(circle in oklab, red, blue)"), probes:[] },
    { name:"gradient-conic", body:gradient("conic-gradient(from 45deg in oklab, red, blue)"), probes:[] },
    { name:"gradient-repeat", body:gradient("repeating-linear-gradient(90deg in oklab, red 0px, blue 16px)"), probes:[] },
    ...["shorter", "longer", "increasing", "decreasing"].map(hue=>({ name:`gradient-oklch-${hue}`, body:gradient(`linear-gradient(90deg in oklch ${hue} hue, red, blue)`), probes:[] })),
  );
  for (const mode of ["multiply","screen","overlay","darken","lighten","color-dodge","color-burn","hard-light","soft-light","difference","exclusion","hue","saturation","color","luminosity"]) {
    cases.push({ name:`linear-${mode}`, body:view("#2080a0")+view("#804020", `,mixBlendMode:"${mode}",mixBlendSpace:"linear",opacity:0.65,width:48,left:8`), probes:[] });
  }
  const exact = process.env.VALLE_COMPOSITING_EXACT?.split(",");
  const only = exact ? "__exact__" : process.env.VALLE_COMPOSITING_ONLY;
  if (only || exact) {
    const selected = cases.filter(fixture => exact
      ? exact.includes(fixture.name)
      : fixture.name === only || fixture.name.startsWith(`${only}-`));
    if (selected.length === 0) throw new Error(`Unknown composition fixture: ${only}`);
    cases.splice(0,cases.length,...selected);
  }
  try {
    for (const fixture of cases) {
      const width = fixture.width ?? 64, height = fixture.height ?? 64;
      const info = { width,height,colorType:ck.ColorType.RGBA_8888,alphaType:ck.AlphaType.Unpremul,colorSpace:ck.ColorSpace.SRGB };
      await writeFile(join(dir,"scene.motion.tsx"), fixture.source ?? `export const composition={width:64,height:64,fps:30,duration:2};
        export default function SceneTest(ctx) { return <Scene style={{width:64,height:64,backgroundColor:"#000"}}>${fixture.body}</Scene>; }`);
      const assetArgs = fixture.atlasAsset ? ["--asset","sprites=atlas-sprites.png"] : [];
      const server = spawn(["motion","studio","scene.motion.tsx",...assetArgs,"--port","0","--web-assets-dir",join(root,"web/dist")]);
      const surface = ck.MakeSurface(width,height)!;
      let engine: ProductEngine | undefined;
      let executor: CanvasKitExecutor | undefined;
      try {
        const ready = await (async () => {
          const reader = server.stdout.getReader();
          let line = "";
          try {
            while (!line.includes("\n")) {
              const part = await reader.read();
              if (part.done) throw new Error(`${fixture.name}: ${await new Response(server.stderr).text()}`);
              line += new TextDecoder().decode(part.value);
            }
          } finally { reader.releaseLock(); }
          return JSON.parse(line.split("\n")[0]!);
        })();
        const config = await (await fetch(new URL("/config.json",ready.url))).json() as Record<string,string>;
        if (!config.fixedPackageManifestJson) throw new Error(`Motion preparation failed: ${JSON.stringify(config)}`);
        engine = new ProductEngine();
        const receipt = JSON.parse(engine.open_fixed_package(config.fixedPackageManifestJson!, config.timelineJson!, config.resourceManifestJson!, config.verifiedBindingBundleJson!));
        executor = new CanvasKitExecutor(ck, {
          transform_srgb_preview_pixels:engine.transform_srgb_preview_pixels.bind(engine),
          apply_bloom_f16:engine.apply_bloom_f16.bind(engine),
          apply_glow_f16:engine.apply_glow_f16.bind(engine),
          apply_radial_blur_f16:engine.apply_radial_blur_f16.bind(engine),
          plan_radial_blur:engine.plan_radial_blur.bind(engine),
          apply_film_grain_f16:engine.apply_film_grain_f16.bind(engine),
          apply_lens_distortion_f16:engine.apply_lens_distortion_f16.bind(engine),
          pack_motion_glass_uniforms:engine.pack_motion_glass_uniforms.bind(engine),
          pack_motion_glass_foreground_uniforms:engine.pack_motion_glass_foreground_uniforms.bind(engine),
        });
        let previous: Uint8Array | undefined;
        let previousNative: Uint8Array<ArrayBuffer> | undefined;
        for (const [request, frame] of (fixture.frames ?? [30,0,30]).entries()) {
          const ticket = engine.evaluate_prepare_preview(receipt.renderId, BigInt(frame),width,height,false);
          let web: Uint8Array;
          try {
            const requests = await decodePackedAbi(engine.resource_requests(ticket), RESOURCE_REQUESTS_ABI) as
              Array<{ handle: number; key: PackedValue; expected: { kind: string } }>;
            const objects = new Map<number, CanvasKitExternalObject>();
            const owned: Array<{ delete(): void }> = [];
            for (const request of requests) {
              if (request.expected.kind === "fontBytes") {
                objects.set(request.handle,{key:request.key,kind:"font",bytes:defaultFont});
              } else if (fixture.atlasAsset && request.expected.kind === "visualFrame") {
                const image = ck.MakeImageFromEncoded(atlasBytes)!;
                owned.push(image);
                objects.set(request.handle,{key:request.key,kind:"visual",image});
              } else {
                throw new Error(`${fixture.name}/${frame}: unexpected external resource ${request.expected.kind}`);
              }
            }
            try {
              engine.lower_canvas_kit(ticket,64n*1024n*1024n,128n*1024n*1024n);
              engine.bind(ticket,1n);
              await executor.execute(engine.plan_template_bytes(ticket),engine.binding_bytes(ticket),engine.bound_schedule_bytes(ticket),
                { generation:1n,objects },{ surface });
              web = Uint8Array.from(surface.getCanvas().readPixels(0,0,info)!);
            } finally { for (const object of owned) object.delete(); }
          } finally { engine.release_ticket(ticket); }
          const output = join(dir,`${fixture.name}-${request}.png`);
          const nativeRun = spawn(["motion","render","scene.motion.tsx",...assetArgs,"--frame",String(frame),"--backend",backend,"-o",output]);
          const [stdout,stderr,code] = await Promise.all([new Response(nativeRun.stdout).text(),new Response(nativeRun.stderr).text(),nativeRun.exited]);
          if (code !== 0) throw new Error(`${fixture.name}: ${stdout}\n${stderr}`);
          const image = ck.MakeImageFromEncoded(await readFile(output))!;
          const native = Uint8Array.from(image.readPixels(0,0,info)!);
          image.delete();
          const rangeSelectorBlurredFront = fixture.name === "range-selector" && frame === 30;
          let maximum = 0, maximumAt = 0;
          let interiorMaximum = 0, totalDifference = 0;
          for (let i=0;i<web.length;i++) {
            const delta = Math.abs(web[i]!-native[i]!);
            if (delta > maximum) { maximum = delta; maximumAt = i; }
            if (rangeSelectorBlurredFront) {
              totalDifference += delta;
              const pixel = Math.floor(i/4), x = pixel%width, y = Math.floor(pixel/width);
              if (!(225 <= x && x <= 320 && 130 <= y && y <= 250)) {
                interiorMaximum = Math.max(interiorMaximum,delta);
              }
            }
          }
          if (rangeSelectorBlurredFront) {
            // CanvasKit and native Skia differ slightly in the Gaussian fringe. Keep the
            // fully revealed letters exact and bound both the fringe and whole-frame error.
            expect(interiorMaximum,"range-selector sharp/interior parity").toBeLessThanOrEqual(2);
            expect(maximum,"range-selector blurred fringe parity").toBeLessThanOrEqual(8);
            expect(totalDifference/(web.length*255),"range-selector mean normalized parity").toBeLessThanOrEqual(0.002);
          } else {
            const at = Math.floor(maximumAt/4)*4;
            const x = Math.floor(maximumAt/4)%width, y = Math.floor(maximumAt/4/width);
            expect(maximum,`${fixture.name}/${frame} parity at (${x},${y}): web ${Array.from(web.slice(at,at+4))}, native ${Array.from(native.slice(at,at+4))}`).toBeLessThanOrEqual(fixture.crossParityTolerance ?? 2);
          }
          if (fixture.name === "geometry-batch") for (const [label,pixels] of [["CanvasKit",web],["Native",native]] as const) {
            let different = 0, firstArea = 0, secondArea = 0;
            for (let dy=-40;dy<40;dy++) for (let dx=-40;dx<40;dx++) {
              const first = ((180+dy)*width+160+dx)*4;
              const second = ((180+dy)*width+480+dx)*4;
              if (Math.max(...[0,1,2].map(c=>Math.abs(pixels[first+c]!-pixels[second+c]!))) > 10) different++;
              if (pixels[first]! > 200 && pixels[first+1]! > 100 && pixels[first+2]! < 30) firstArea++;
              if (pixels[second]! > 200 && pixels[second+1]! > 100 && pixels[second+2]! < 30) secondArea++;
            }
            expect(different,`${label} geometry-batch rotated pixels`).toBeGreaterThanOrEqual(100);
            expect(Math.abs(firstArea-secondArea)/Math.max(firstArea,secondArea),`${label} geometry-batch filled area`).toBeLessThanOrEqual(0.03);
          }
          if (fixture.name === "oklch-color-interpolation-text" && frame === 30) for (const [label,pixels] of [["CanvasKit",web],["Native",native]] as const) {
            for (const [name,top,bottom] of [["plain",0,180],["units",180,360]] as const) {
              let teal = 0;
              for (let i=top*width*4;i<bottom*width*4;i+=4)
                if (pixels[i]! < 30 && pixels[i+1]! > 140 && pixels[i+2]! > 100) teal++;
              expect(teal,`${label} oklch-color-interpolation ${name} float text area`).toBeGreaterThan(500);
            }
          }
          if (fixture.name === "oklch-color-interpolation-text-stroke" && frame === 30) for (const [label,pixels] of [["CanvasKit",web],["Native",native]] as const) {
            let teal = 0;
            for (let i=0;i<pixels.length;i+=4)
              if (pixels[i]! < 40 && pixels[i+1]! > 140 && pixels[i+2]! > 90) teal++;
            expect(teal,`${label} oklch-color-interpolation float text stroke area`).toBeGreaterThan(500);
          }
          if (fixture.name === "batch-skew-stroke") for (const [label,pixels] of [["CanvasKit",web],["Native",native]] as const) {
            let different = 0;
            for (let y=12;y<28;y++) for (let dx=0;dx<16;dx++) {
              const first = (y*width+8+dx)*4;
              const second = (y*width+40+dx)*4;
              if ([0,1,2].some(c=>Math.abs(pixels[first+c]!-pixels[second+c]!)>10)) different++;
            }
            if (frame === 0) expect(different,`${label} batch shear/stroke zero control`).toBe(0);
            else expect(different,`${label} batch shear/stroke changes pixels`).toBeGreaterThanOrEqual(10);
          }
          if (fixture.name === "easing-presets") {
            const leftEdge = (y:number) => {
              for (let x=0;x<width;x++) if (native[(y*width+x)*4]! > 240) return x;
              throw new Error(`easing-presets missing rectangle at row ${y}`);
            };
            easingPositions.push({frame,back:leftEdge(100),steps:leftEdge(260)});
          }
          for (const [x,y,r,g,b,a] of [...fixture.probes,...(fixture.frameProbes?.[frame] ?? [])]) for (const pixels of [web,native]) {
            for (const [channel,expected] of [r,g,b].entries())
              expect(Math.abs(pixels[(y*width+x)*4+channel]!-expected),`${fixture.name}/${frame} (${x},${y}) channel ${channel}`).toBeLessThanOrEqual(2);
            if (a !== undefined) expect(Math.abs(pixels[(y*width+x)*4+3]!-a),`${fixture.name}/${frame} alpha`).toBeLessThanOrEqual(1);
          }
          if (fixture.name.startsWith("geometry-batch-atlas") && fixture.name !== "geometry-batch-atlas-right") for (const [label,pixels] of [["CanvasKit",web],["Native",native]] as const) {
            let magentaLeak = 0;
            for (let i=0;i<pixels.length;i+=4) magentaLeak = Math.max(magentaLeak,pixels[i]!-pixels[i+1]!);
            expect(magentaLeak,`${label} ${fixture.name} samples only the left atlas tile`).toBeLessThanOrEqual(2);
          }
          if ((frame === 30 && fixture.name !== "path-modifiers-extra" && fixture.name !== "instances-circle") || (fixture.name === "path-modifiers-extra" && frame === 0) || ((fixture.name === "path-modifiers" || fixture.name === "rich-text-units") && frame === 10) || (fixture.name === "reverse-path-trail" && frame === 0) || (fixture.name === "particle-forces" && frame === 45) || (fixture.name === "auto-motion-blur" && frame === 10) || (fixture.name === "shutter-sampling" && frame === 20) || (fixture.name === "instances-shutter" && frame === 20) || (fixture.name === "instances-circle" && frame === 15) || (fixture.name === "time-scope" && frame === 15) || (fixture.name === "spring-simulation" && frame === 15) || (fixture.name === "allow-self-intersection" && frame === 27)) {
            if (previous) {
              const expected=previous;
              expect(web.length===expected.length && web.every((value,index)=>value===expected[index]),`${fixture.name} random access`).toBe(true);
            }
            if ((fixture.name === "spring-simulation" || fixture.name === "instances-shutter" || fixture.name === "instances-circle" || fixture.name === "allow-self-intersection") && previousNative) {
              const expected=previousNative;
              expect(native.length===expected.length && native.every((value,index)=>value===expected[index]),`${fixture.name} native random access`).toBe(true);
            }
            previous = web;
            previousNative = native;
          }
          images.set(fixture.name,web);
          frameImages.set(`${fixture.name}/${frame}`, { web, native });
        }
      } catch (error) {
        throw new Error(`${fixture.name}: ${error instanceof Error ? error.message : String(error)}`, { cause:error });
      } finally {
        server.kill(); await server.exited;
        executor?.dispose(); engine?.free(); surface.delete();
      }
    }
    if (!only || only === "scene-bloom") for (const label of ["native","web"] as const) {
      const pixel = (name:string,x:number,y:number) => {
        const image = frameImages.get(`${name}/0`)![label];
        const at = (y*640+x)*4;
        return [image[at]!,image[at+1]!,image[at+2]!] as const;
      };
      const brightness = (rgb:readonly number[]) => Math.round(0.2126*rgb[0]!+0.7152*rgb[1]!+0.0722*rgb[2]!);
      const hot = brightness(pixel("scene-bloom",210,180));
      const dim = brightness(pixel("scene-bloom",550,180));
      expect(hot,`${label} scene-bloom white edge +30`).toBeGreaterThanOrEqual(30);
      expect(dim,`${label} scene-bloom gray edge +30`).toBeLessThanOrEqual(20);
      expect(hot,`${label} scene-bloom intensity control`).toBeGreaterThan(brightness(pixel("scene-bloom-half",210,180)));
      expect(brightness(pixel("scene-bloom-no-filter",210,180)),`${label} scene-bloom filter control`).toBeLessThanOrEqual(20);
    }
    if (!only || only === "radial-blur") for (const label of ["native","web"] as const) {
      const red = (name:string,x:number,y:number) =>
        frameImages.get(`${name}/0`)![label][(y*640+x)*4]!;
      expect(red("radial-blur",370,180),`${label} radial-blur right radial tail`).toBeGreaterThanOrEqual(30);
      expect(red("radial-blur",270,180),`${label} radial-blur left radial tail`).toBeGreaterThanOrEqual(30);
      expect(red("radial-blur",320,150),`${label} radial-blur upper radial tail`).toBeGreaterThanOrEqual(30);
      expect(red("radial-blur",390,180),`${label} radial-blur finite extent`).toBeLessThanOrEqual(20);
      expect(red("radial-blur",320,180),`${label} radial-blur center stays bright`).toBeGreaterThanOrEqual(250);
      expect(red("radial-blur",370,180),`${label} radial-blur amount control`).toBeGreaterThan(red("radial-blur-half",370,180));
      expect(red("radial-blur-no-filter",370,180),`${label} radial-blur no-filter control`).toBeLessThanOrEqual(20);
    }
    if (!only || only === "film-grain") for (const label of ["native","web"] as const) {
      const image = (name:string) => frameImages.get(`${name}/0`)![label];
      const red = (name:string,x:number,y:number) => image(name)[(y*640+x)*4]!;
      const values = Array.from({length:160},(_,i)=>red("film-grain",230+i,180));
      expect(Math.min(...values),`${label} film-grain dark grains`).toBeLessThan(118);
      expect(Math.max(...values),`${label} film-grain light grains`).toBeGreaterThan(138);
      expect(new Set(values).size,`${label} film-grain tonal variation`).toBeGreaterThan(15);
      expect(Math.abs(red("film-grain-zero",320,180)-red("film-grain-no-filter",320,180)),`${label} film-grain zero amount`).toBeLessThanOrEqual(2);
      expect(red("film-grain",100,180),`${label} film-grain outside node`).toBe(16);
      let changed = 0;
      for (let x=230;x<390;x++) if (red("film-grain",x,180)!==red("film-grain-seed",x,180)) changed++;
      expect(changed,`${label} film-grain seed changes pattern`).toBeGreaterThan(100);
      expect(image("film-grain"),`${label} film-grain changes each frame`).not.toEqual(frameImages.get("film-grain/30")![label]);
    }
    if (!only || only === "lens-distortion") for (const label of ["native","web"] as const) {
      const image = (name:string) => frameImages.get(`${name}/0`)![label];
      const red = (name:string,x:number,y:number) => image(name)[(y*640+x)*4]!;
      const leftEdge = (name:string) => {
        for (let x=200;x<320;x++) if (red(name,x,180)>=200) return x;
        throw new Error(`${label} ${name} left bar is missing`);
      };
      const rightEdge = (name:string) => {
        for (let x=439;x>=320;x--) if (red(name,x,180)>=200) return x;
        throw new Error(`${label} ${name} right bar is missing`);
      };
      expect(leftEdge("lens-distortion"),`${label} lens-distortion left radial shift`).toBeLessThanOrEqual(leftEdge("lens-distortion-no-filter")-4);
      expect(rightEdge("lens-distortion"),`${label} lens-distortion right radial shift`).toBeGreaterThanOrEqual(rightEdge("lens-distortion-no-filter")+4);
      expect(Math.abs(leftEdge("lens-distortion-zero")-leftEdge("lens-distortion-no-filter")),`${label} lens-distortion zero k1/k2`).toBeLessThanOrEqual(1);
      expect(leftEdge("lens-distortion-k2"),`${label} lens-distortion k2 changes the lens`).toBeGreaterThan(leftEdge("lens-distortion"));
      expect(red("lens-distortion",190,180),`${label} lens-distortion outside frame`).toBe(16);
      const cropRed = (name:string,x:number,y:number) => image(name)[(y*320+x)*4]!;
      expect(cropRed("lens-distortion-crop",316,180),`${label} lens-distortion samples past viewport edge`).toBeGreaterThanOrEqual(200);
      expect(cropRed("lens-distortion-crop-no-filter",316,180),`${label} lens-distortion viewport crop control`).toBeLessThan(60);
    }
    if (!only || only === "instances-shutter") {
      for (const frame of [0,20,40]) {
        const batched = frameImages.get(`instances-shutter/${frame}`)!;
        const expanded = frameImages.get(`instances-shutter-expanded/${frame}`)!;
        expect(batched.native,`instances shutter batched/expanded native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<batched.web.length;index++)
          maximum = Math.max(maximum,Math.abs(batched.web[index]!-expanded.web[index]!));
        expect(maximum,`instances shutter batched/expanded CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-circle") {
      for (const name of ["instances-circle","instances-circle-varied"]) for (const frame of [0,15,30]) {
        const templated = frameImages.get(`${name}/${frame}`)!;
        const expanded = frameImages.get(`${name}-expanded/${frame}`)!;
        expect(templated.native,`${name} native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`${name} CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-path") {
      for (const frame of [0,15,30]) {
        const templated = frameImages.get(`instances-path/${frame}`)!;
        const expanded = frameImages.get(`instances-path-expanded/${frame}`)!;
        expect(templated.native,`instances-path native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-path CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-path" || only === "instances-path-opacity") {
      for (const frame of [0,15,30]) {
        const templated = frameImages.get(`instances-path-opacity/${frame}`)!;
        const expanded = frameImages.get(`instances-path-opacity-expanded/${frame}`)!;
        expect(templated.native,`instances-path-opacity native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-path-opacity CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-path" || only === "instances-path-rotation") {
      for (const frame of [0,15,30]) {
        const templated = frameImages.get(`instances-path-rotation/${frame}`)!;
        const expanded = frameImages.get(`instances-path-rotation-expanded/${frame}`)!;
        expect(templated.native,`instances-path-rotation native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-path-rotation CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    for (const name of ["instances-path-scale", "instances-path-scale-mirror", "instances-path-skew", "instances-path-stroke", "instances-path-stroke-dynamic", "instances-path-stroke-colors", "instances-path-stroke-colors-dynamic", "instances-path-stroke-only", "instances-path-stroke-style", "instances-path-stroke-miter", "instances-path-stroke-style-colors", "instances-path-scale-rotation", "instances-path-scale-rotation-skew", "instances-path-stroke-affine", "instances-path-stroke-affine-opaque", "instances-path-stroke-curve"]) {
      if (only && only !== "instances-path" && only !== "instances-path-scale" && only !== name) continue;
      for (const frame of [0,15,30]) {
        const templated = frameImages.get(`${name}/${frame}`)!;
        const expanded = frameImages.get(`${name}-expanded/${frame}`)!;
        if (name === "instances-path-stroke-style" || name === "instances-path-stroke-miter" || name === "instances-path-stroke-style-colors") {
          // Fractional translation of dashed stroke edges changes a few antialiased channels
          // by at most five in both Skia runtimes; every other Path case remains exact here.
          let nativeMaximum = 0;
          for (let index=0;index<templated.native.length;index++) {
            const difference = Math.abs(templated.native[index]!-expanded.native[index]!);
            nativeMaximum = Math.max(nativeMaximum,difference);
          }
          expect(nativeMaximum,`${name} native frame ${frame}`).toBeLessThanOrEqual(5);
        } else {
          expect(templated.native,`${name} native frame ${frame}`).toEqual(expanded.native);
        }
        let maximum = 0;
        let visiblePixels = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        for (let index=0;index<templated.native.length;index+=4)
          if (templated.native[index] !== 16 || templated.native[index+1] !== 16 || templated.native[index+2] !== 16)
            visiblePixels++;
        expect(maximum,`${name} CanvasKit frame ${frame}`).toBeLessThanOrEqual(name === "instances-path-stroke-style" || name === "instances-path-stroke-miter" || name === "instances-path-stroke-style-colors" ? 5 : name.startsWith("instances-path-stroke-colors") || name === "instances-path-stroke-only" ? 3 : 2);
        expect(visiblePixels,`${name} visible paths frame ${frame}`).toBeGreaterThan(100);
      }
    }
    if (!only || only === "instances-flow") {
      for (const frame of [0,30]) {
        const templated = frameImages.get(`instances-flow/${frame}`)!;
        const expanded = frameImages.get(`instances-flow-expanded/${frame}`)!;
        expect(templated.native,`instances-flow native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-flow CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
        for (const [label,pixels] of [["Native",templated.native],["CanvasKit",templated.web]] as const) {
          const blue = pixels[(4*160+30)*4+2]!;
          if (frame === 0) expect(blue,`${label} first flow row initially painted`).toBeGreaterThan(100);
          else expect(blue,`${label} hidden flow row retains its box without paint`).toBeLessThan(30);
        }
      }
    }
    if (!only || only === "instances-cards") {
      for (const frame of [0,30]) {
        const templated = frameImages.get(`instances-cards/${frame}`)!;
        const expanded = frameImages.get(`instances-cards-expanded/${frame}`)!;
        expect(templated.native,`instances-cards native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-cards CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-component") {
      for (const frame of [0,30]) {
        const templated = frameImages.get(`instances-component/${frame}`)!;
        const expanded = frameImages.get(`instances-component-expanded/${frame}`)!;
        expect(templated.native,`instances-component native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-component CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-group") {
      for (const name of ["instances-group", "instances-group-component"]) for (const frame of [0,30]) {
        const templated = frameImages.get(`${name}/${frame}`)!;
        const expanded = frameImages.get(`${name}-expanded/${frame}`)!;
        expect(templated.native,`${name} native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`${name} CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    if (!only || only === "instances-text-root") {
      for (const name of ["instances-text-root", "instances-text-root-direct"]) for (const frame of [0,30]) {
        const templated = frameImages.get(`${name}/${frame}`)!;
        const expanded = frameImages.get(`${name}-expanded/${frame}`)!;
        expect(templated.native,`${name} native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`${name} CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
        for (const [label,pixels] of [["Native",templated.native],["CanvasKit",templated.web]] as const) {
          let colored = 0;
          for (let index=0;index<pixels.length;index+=4)
            if (pixels[index]! > 32 || pixels[index+1]! > 32 || pixels[index+2]! > 32) colored++;
          expect(colored,`${name} ${label} visible text frame ${frame}`).toBeGreaterThan(60);
        }
      }
    }
    if (!only || only === "instances-cards-classes") {
      for (const frame of [0,30]) {
        const templated = frameImages.get(`instances-cards-classes/${frame}`)!;
        const expanded = frameImages.get(`instances-cards-classes-expanded/${frame}`)!;
        expect(templated.native,`instances-cards-classes native frame ${frame}`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<templated.web.length;index++)
          maximum = Math.max(maximum,Math.abs(templated.web[index]!-expanded.web[index]!));
        expect(maximum,`instances-cards-classes CanvasKit frame ${frame}`).toBeLessThanOrEqual(2);
      }
    }
    for (const [name,frames] of [["instances",[0,15,30]],["oklch-color-interpolation-instances",[0,30,59]]] as const) {
      if (!exact?.includes(name) || !exact.includes(`${name}-expanded`)) continue;
      for (const frame of frames) {
        const instanced = frameImages.get(`${name}/${frame}`)!;
        const expanded = frameImages.get(`${name}-expanded/${frame}`)!;
        expect(instanced.native,`${name}/${frame} CPU pixel equivalence`).toEqual(expanded.native);
        let maximum = 0;
        for (let index=0;index<instanced.web.length;index++)
          maximum = Math.max(maximum,Math.abs(instanced.web[index]!-expanded.web[index]!));
        expect(maximum,`${name}/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
      }
    }
    if ((only || exact) && only !== "node-glow") return;
    for (const label of ["native","web"] as const) {
      const pixel = (name:string,x:number,y:number) => {
        const image = frameImages.get(`${name}/0`)![label];
        const at = (y*640+x)*4;
        return [image[at]!,image[at+1]!,image[at+2]!] as const;
      };
      const luminance = ([r,g,b]:readonly [number,number,number]) =>
        Math.round(0.2126*r+0.7152*g+0.0722*b);
      const near = pixel("node-glow",380,180);
      expect(luminance(near),`${label} node-glow near glow`).toBeGreaterThanOrEqual(40);
      const decay = [380,390,400,420,480].map(x=>luminance(pixel("node-glow",x,180)));
      for (let index=1;index<decay.length;index++)
        expect(decay[index]!,`${label} node-glow monotonic halo at ${[380,390,400,420,480][index]}`).toBeLessThanOrEqual(decay[index-1]!);
      expect(luminance(pixel("node-glow",480,180)),`${label} node-glow far decay`).toBeLessThanOrEqual(22);
      expect(near[1],`${label} node-glow cyan hue`).toBeGreaterThan(near[0]+10);
      expect(near[2],`${label} node-glow blue hue`).toBeGreaterThanOrEqual(near[1]-2);
      expect(luminance(near),`${label} node-glow intensity control`).toBeGreaterThan(luminance(pixel("node-glow-half",380,180)));
      expect(luminance(pixel("node-glow-no-filter",380,180)),`${label} node-glow filter control`).toBeLessThanOrEqual(22);
    }
    if (only === "node-glow") return;
    for (const label of ["native","web"] as const) {
      const edge = (name:string, channel:number) => {
        const pixels = frameImages.get(`${name}/0`)![label];
        for (let x=180;x<460;x++) if (pixels[(180*640+x)*4+channel]! >= 128) return x;
        throw new Error(`${label} ${name} channel ${channel} has no edge`);
      };
      expect(Math.abs(edge("chromatic-aberration",0)-edge("chromatic-aberration",2)),`${label} chromatic-aberration channel separation`).toBeGreaterThanOrEqual(4);
      expect(edge("chromatic-aberration-no-filter",0),`${label} chromatic-aberration control red/blue edge`).toBe(edge("chromatic-aberration-no-filter",2));
    }
    for (const label of ["native","web"] as const) {
      const count = (name:string,frame:number) => {
        const pixels=frameImages.get(`${name}/${frame}`)![label];
        let bright=0;
        for(let i=0;i<pixels.length;i+=4) if(Math.max(pixels[i]!,pixels[i+1]!,pixels[i+2]!)>80)bright++;
        return bright;
      };
      const at=[0,6,30,54,59].map(frame=>count("text-outline",frame));
      expect(at[0],`${label} text-outline frame 0`).toBeLessThan(50);
      expect(at[1],`${label} text-outline draw-on 6→30`).toBeLessThan(at[2]!);
      expect(at[2],`${label} text-outline draw-on 30→54`).toBeLessThan(at[3]!);
      const control=count("text-outline-control",0);
      expect(Math.abs(at[4]!-control)/control,`${label} text-outline stroke parity`).toBeLessThanOrEqual(0.1);
      expect(frameImages.get("text-outline-fill-path/0")![label],`${label} text-outline filled glyph parity`)
        .toEqual(frameImages.get("text-outline-fill-text/0")![label]);
    }
    for (const label of ["native","web"] as const) {
      const pixels=(name:string,frame:number)=>frameImages.get(`${name}/${frame}`)![label];
      const red=(image:Uint8Array,x:number,y:number)=>image[(y*640+x)*4]!;
      const brightest=(image:Uint8Array,x0:number,x1:number)=>{
        let value=0;for(let y=130;y<250;y++)for(let x=x0;x<x1;x++)value=Math.max(value,red(image,x,y));
        return value;
      };
      const selected=pixels("range-selector",30), sharp=pixels("range-selector-no-blur",30), full=pixels("range-selector-full",30);
      expect(brightest(pixels("range-selector",0),40,601),`${label} range-selector starts hidden`).toBeLessThan(60);
      expect(brightest(selected,40,201),`${label} range-selector left`).toBeGreaterThan(200);
      expect(brightest(selected,460,601),`${label} range-selector right`).toBeLessThan(60);
      const opacity=(brightest(selected,246,297)-16)/(brightest(full,246,297)-16);
      expect(opacity,`${label} range-selector frontier opacity`).toBeGreaterThan(0.2);
      expect(opacity,`${label} range-selector frontier opacity`).toBeLessThan(0.8);
      const band=(image:Uint8Array,x0:number,x1:number,high:number)=>{
        let count=0;for(let x=x0;x<x1;x++)if(red(image,x,190)>=40&&red(image,x,190)<=high)count++;
        return count;
      };
      expect(band(selected,238,265,160),`${label} range-selector feathered C edge`).toBeGreaterThanOrEqual(4);
      expect(band(sharp,238,265,160),`${label} range-selector sharp negative control`).toBeLessThanOrEqual(2);
      expect(band(selected,196,205,220),`${label} range-selector revealed E edge`).toBeLessThanOrEqual(2);
    }
    for (const label of ["native","web"] as const) {
      const red=(image:Uint8Array,x:number,y:number)=>{
        const at=(y*640+x)*4;
        return image[at]!>180 && image[at+1]!<120;
      };
      const redCount=(image:Uint8Array)=>{
        let count=0;for(let y=130;y<250;y++)for(let x=230;x<450;x++)if(red(image,x,y))count++;
        return count;
      };
      const shifts=(name:string)=>{
        const before=frameImages.get(`${name}/10`)![label];
        const after=frameImages.get(`${name}/20`)![label];
        const columns:number[]=[];
        for(let x=230;x<450;x++){
          let occupied=false;
          for(let y=130;y<250;y++)if(red(before,x,y)||red(after,x,y)){occupied=true;break;}
          if(occupied)columns.push(x);
        }
        const ranges:Array<[number,number]>=[];
        for(const x of columns){
          if(!ranges.length || x>ranges[ranges.length-1]![1]+1)ranges.push([x,x]);
          else ranges[ranges.length-1]![1]=x;
        }
        expect(ranges.length,`${label} ${name} red glyph columns`).toBe(5);
        const top=(image:Uint8Array,from:number,to:number)=>{
          for(let y=130;y<250;y++)for(let x=from;x<=to;x++)if(red(image,x,y))return y;
          throw new Error(`${label} ${name} has no red glyph in ${from}..${to}`);
        };
        return ranges.map(([from,to])=>top(after,from,to)-top(before,from,to));
      };
      for(const frame of [10,20]){
        expect(redCount(frameImages.get(`rich-text-units/${frame}`)![label]),`${label} rich-text-units red Span/${frame}`).toBeGreaterThanOrEqual(200);
      }
      const wave=shifts("rich-text-units"), uniform=shifts("rich-text-units-uniform");
      const spread=(values:number[])=>Math.max(...values)-Math.min(...values);
      expect(spread(wave),`${label} rich-text-units per-letter phase`).toBeGreaterThanOrEqual(4);
      expect(spread(uniform),`${label} rich-text-units uniform-phase negative control`).toBeLessThanOrEqual(2);
    }
    for (const label of ["native","web"] as const) {
      const pixels=frameImages.get("multiline-path-text/0")![label];
      const radial=[0,0,0];
      let bright=0;
      const ink=(x:number,y:number)=>{
        const at=(y*640+x)*4;
        return Math.max(pixels[at]!,pixels[at+1]!,pixels[at+2]!)>80;
      };
      const radius=(x:number,y:number)=>Math.round(Math.hypot(x-320,y-340));
      for(let y=0;y<360;y++)for(let x=0;x<640;x++)if(ink(x,y)){
        bright++;
        const r=radius(x,y);
        if(r>=224&&r<245)radial[0]!++;
        else if(r>=246&&r<260)radial[1]!++;
        else if(r>=260&&r<282)radial[2]!++;
      }
      expect(bright,`${label} multiline-path-text visible text`).toBeGreaterThanOrEqual(1500);
      expect(radial[0],`${label} multiline-path-text inner line`).toBeGreaterThanOrEqual(1000);
      expect(radial[2],`${label} multiline-path-text outer line`).toBeGreaterThanOrEqual(1000);
      expect(radial[1],`${label} multiline-path-text radial gap`).toBeLessThanOrEqual(40);
      const top=(from:number,to:number,minR:number,maxR:number)=>{
        for(let y=0;y<360;y++)for(let x=from;x<to;x++)
          if(ink(x,y)&&radius(x,y)>=minR&&radius(x,y)<maxR)return y;
        throw new Error(`${label} multiline-path-text missing arc ink in ${from}..${to}`);
      };
      expect(top(60,120,260,282)-top(200,260,260,282),`${label} multiline-path-text outer arc rise`).toBeGreaterThanOrEqual(20);
      expect(top(90,150,224,245)-top(220,280,224,245),`${label} multiline-path-text inner arc rise`).toBeGreaterThanOrEqual(20);
    }
    for (const label of ["native","web"] as const) {
      const area=(frame:number)=>{
        const pixels=frameImages.get(`star-to-circle-morph/${frame}`)![label];
        let count=0;
        for(let at=0;at<pixels.length;at+=4)if(pixels[at]!>128)count++;
        return count;
      };
      const areas=[0,30,59].map(area);
      expect(areas[0],`${label} star-to-circle-morph star-to-mid area`).toBeLessThan(areas[1]!);
      expect(areas[1],`${label} star-to-circle-morph mid-to-circle area`).toBeLessThan(areas[2]!);
      const mid=frameImages.get("star-to-circle-morph/30")![label];
      const red=(x:number,y:number)=>mid[(y*640+x)*4]!;
      let partial=0, interiorPartial=0;
      for(let y=1;y<359;y++)for(let x=1;x<639;x++)if(red(x,y)>=40&&red(x,y)<=215){
        partial++;
        if([[1,0],[-1,0],[0,1],[0,-1]].every(([dx,dy])=>{
          const value=red(x+dx!,y+dy!);
          return value>=40&&value<=215;
        }))interiorPartial++;
      }
      expect(partial,`${label} star-to-circle-morph no broad cross-fade`).toBeLessThanOrEqual(3000);
      expect(interiorPartial,`${label} star-to-circle-morph partial pixels stay on edges`).toBeLessThanOrEqual(20);
    }
    for (const label of ["native","web"] as const) {
      const middle=frameImages.get("path-morph-sequence/30")![label];
      const square=frameImages.get("path-morph-sequence-control/30")![label];
      expect(middle,`${label} path-morph-sequence middle stop equals the static square`).toEqual(square);
      const area=(frame:number)=>{
        const pixels=frameImages.get(`path-morph-sequence/${frame}`)![label];
        let count=0;
        for(let at=0;at<pixels.length;at+=4)if(pixels[at]!>128)count++;
        return count;
      };
      expect(Math.abs(area(45)-area(15)),`${label} path-morph-sequence distinct transitions`).toBeGreaterThanOrEqual(1000);
    }
    for (const label of ["native","web"] as const) {
      const pixels=(frame:number)=>frameImages.get(`convex-morph/${frame}`)![label];
      const topBounds=(frame:number)=>{
        const image=pixels(frame);
        let minX=640,maxX=-1,area=0,partial=0,interiorPartial=0;
        const red=(x:number,y:number)=>image[(y*640+x)*4]!;
        for(let y=15;y<175;y++)for(let x=0;x<640;x++){
          const value=red(x,y);
          if(value>128){area++;minX=Math.min(minX,x);maxX=Math.max(maxX,x);}
          if(value>=40&&value<=215){
            partial++;
            if(x>0&&x<639&&y>15&&y<174&&[[1,0],[-1,0],[0,1],[0,-1]].every(([dx,dy])=>{
              const neighbor=red(x+dx!,y+dy!);
              return neighbor>=40&&neighbor<=215;
            }))interiorPartial++;
          }
        }
        return {minX,maxX,area,partial,interiorPartial};
      };
      const start=topBounds(0),middle=topBounds(30),end=topBounds(59);
      expect(start.minX,`${label} convex start x`).toBeGreaterThanOrEqual(49);
      expect(start.maxX,`${label} convex start width`).toBeLessThanOrEqual(151);
      expect(middle.minX,`${label} convex middle x`).toBeGreaterThanOrEqual(238);
      expect(middle.maxX,`${label} convex middle width`).toBeLessThanOrEqual(357);
      expect(end.minX,`${label} convex end x`).toBeGreaterThanOrEqual(420);
      expect(end.maxX,`${label} convex end width`).toBeLessThanOrEqual(560);
      expect(middle.area,`${label} convex middle fill`).toBeGreaterThan(6000);
      expect(middle.partial,`${label} convex no broad cross-fade`).toBeLessThanOrEqual(1200);
      expect(middle.interiorPartial,`${label} convex partial pixels stay on edges`).toBeLessThanOrEqual(20);
      const key=pixels(30),control=frameImages.get("convex-control/30")![label];
      expect(key.length===control.length&&key.every((value,index)=>value===control[index]),
        `${label} convex sequence middle stop equals static diamond`).toBe(true);
      const cyanArea=(frame:number)=>{
        const image=pixels(frame);
        let area=0;
        for(let y=185;y<350;y++)for(let x=0;x<640;x++){
          const at=(y*640+x)*4;
          if(image[at+1]!>140&&image[at+2]!>170)area++;
        }
        return area;
      };
      expect(Math.abs(cyanArea(15)-cyanArea(45)),`${label} convex sequence changes shape`).toBeGreaterThan(1500);
    }
    for (const label of ["native","web"] as const) {
      const pixels=(frame:number)=>frameImages.get(`arc-length-morph/${frame}`)![label];
      const top=(frame:number)=>{
        const image=pixels(frame);
        let count=0,minX=640,maxX=-1,partial=0;
        for(let y=20;y<140;y++)for(let x=0;x<640;x++){
          const value=image[(y*640+x)*4]!;
          if(value>128){count++;minX=Math.min(minX,x);maxX=Math.max(maxX,x);}
          if(value>=40&&value<=215)partial++;
        }
        return {count,minX,maxX,partial};
      };
      const start=top(0),middle=top(30),end=top(59);
      expect(start.minX,`${label} arc length start x`).toBeGreaterThanOrEqual(39);
      expect(start.maxX,`${label} arc length start width`).toBeLessThanOrEqual(141);
      expect(middle.minX,`${label} arc length middle x`).toBeGreaterThanOrEqual(188);
      expect(middle.maxX,`${label} arc length middle width`).toBeLessThanOrEqual(297);
      expect(end.minX,`${label} arc length end x`).toBeGreaterThanOrEqual(334);
      expect(end.maxX,`${label} arc length end width`).toBeLessThanOrEqual(452);
      expect(middle.count,`${label} arc length middle filled contour`).toBeGreaterThan(4500);
      expect(middle.partial,`${label} arc length no broad cross-fade`).toBeLessThanOrEqual(1200);
      const key=pixels(30),control=frameImages.get("arc-length-control/30")![label];
      expect(key.length===control.length&&key.every((value,index)=>value===control[index]),
        `${label} arc length sequence key equals static resampled contour`).toBe(true);
    }
    for (const label of ["native","web"] as const) {
      const key=frameImages.get("compatible-morph/30")![label];
      const control=frameImages.get("compatible-control/30")![label];
      expect(key.length===control.length&&key.every((value,index)=>value===control[index]),
        `${label} compatible sequence key equals authored contour`).toBe(true);
    }
    for (const label of ["native","web"] as const) {
      const areas=(frame:number)=>{
        const pixels=frameImages.get(`anchor-rotation/${frame}`)![label];
        let anchored=0,automatic=0;
        for(let y=0;y<240;y++)for(let x=0;x<640;x++){
          const at=(y*640+x)*4;
          if(y<120&&pixels[at]!>128&&pixels[at+1]!>128&&pixels[at+2]!>128)anchored++;
          if(y>=120&&pixels[at+1]!>140&&pixels[at+2]!>170)automatic++;
        }
        return {anchored,automatic};
      };
      const start=areas(0),middle=areas(30);
      expect(Math.abs(start.anchored-start.automatic),`${label} anchor rotation matching start area`).toBeLessThan(300);
      expect(middle.anchored,`${label} anchor rotation keeps a filled contour`).toBeGreaterThan(2500);
      expect(middle.automatic-middle.anchored,`${label} authored anchor changes the correspondence`).toBeGreaterThan(2000);
    }
    for (const label of ["native","web"] as const) {
      const cyanArea=(frame:number)=>{
        const pixels=frameImages.get(`paired-contours/${frame}`)![label];
        let area=0;
        for(let y=190;y<330;y++)for(let x=390;x<530;x++){
          const at=(y*640+x)*4;
          if(pixels[at+1]!>140&&pixels[at+2]!>170)area++;
        }
        return area;
      };
      expect(cyanArea(0)-cyanArea(30),`${label} paired hole opens at middle key`).toBeGreaterThan(1300);
      expect(cyanArea(59)-cyanArea(30),`${label} paired hole closes again`).toBeGreaterThan(1200);
    }
    for (const label of ["native","web"] as const) {
      for (const frame of [0,30,59]) {
        const pixels=frameImages.get(`paired-anchors/${frame}`)![label];
        const control=frameImages.get(`paired-contours/${frame}`)![label];
        let anchored=0,ordinary=0;
        for(let at=0;at<pixels.length;at+=4){
          if(pixels[at]!>128&&pixels[at+1]!>128&&pixels[at+2]!>128)anchored++;
          if(control[at]!>128&&control[at+1]!>128&&control[at+2]!>128)ordinary++;
        }
        expect(Math.abs(anchored-ordinary),`${label} paired anchors retain contour area at ${frame}`).toBeLessThan(80);
      }
    }
    for (const label of ["native","web"] as const) {
      for (const [automatic,explicit] of [["auto-contours","paired-contours"],["auto-anchors","paired-anchors"]]) {
        for (const frame of [0,30,59]) {
          expect(frameImages.get(`${automatic}/${frame}`)![label],`${label} ${automatic} infers the explicit correspondence at ${frame}`)
            .toEqual(frameImages.get(`${explicit}/${frame}`)![label]);
        }
      }
    }
    for (const label of ["native","web"] as const) {
      for (const frame of [0,27,59]) {
        const pixels=frameImages.get(`allow-self-intersection/${frame}`)![label];
        let filled=0;
        for(let at=0;at<pixels.length;at+=4)if(pixels[at]!>128)filled++;
        expect(filled,`${label} explicit self-intersection frame ${frame} draws geometry`).toBeGreaterThan(20);
      }
    }
    for (const label of ["native","web"] as const) {
      const reverse=frameImages.get("reverse-path-trail/15")![label];
      const forward=frameImages.get("reverse-path-trail-forward/15")![label];
      const red=(pixels:Uint8Array,x:number,y:number)=>pixels[(y*640+x)*4]!;
      expect(red(reverse,363,47),`${label} reverse-path-trail reverse head above orbit`).toBeGreaterThanOrEqual(250);
      expect(red(forward,363,313),`${label} reverse-path-trail forward head below orbit`).toBeGreaterThanOrEqual(250);
      expect(red(reverse,410,70),`${label} reverse-path-trail negative-gap tail behind head`).toBeGreaterThanOrEqual(140);
      expect(red(reverse,410,70),`${label} reverse-path-trail tail is dimmer than head`).toBeLessThan(red(reverse,363,47));
    }
    for (const label of ["native","web"] as const) {
      const profile=(frame:number)=>{
        const pixels=frameImages.get(`path-modifiers/${frame}`)![label];
        const red=(x:number,y:number)=>pixels[(y*640+x)*4]!;
        let area=0, xsum=0, ysum=0;
        for(let y=140;y<350;y++)for(let x=365;x<585;x++){
          const weight=Math.min(1,Math.max(0,(red(x,y)-16)/228));
          if(weight<=0.05)continue;
          area+=weight;xsum+=x*weight;ysum+=y*weight;
        }
        const cx=xsum/area,cy=ysum/area;
        const radii=Array.from({length:36},(_,index)=>{
          const angle=index*Math.PI/18;
          let last=0;
          for(let radius=40;radius<110;radius++){
            const x=Math.round(cx+radius*Math.cos(angle));
            const y=Math.round(cy+radius*Math.sin(angle));
            if(red(x,y)>100)last=radius;
          }
          return last;
        });
        return {area,cx,cy,radii};
      };
      const first=profile(10),later=profile(40);
      expect(Math.hypot(first.cx-later.cx,first.cy-later.cy),`${label} path-modifiers centered deformation`).toBeLessThanOrEqual(2);
      expect(Math.abs(first.area-later.area)/first.area,`${label} path-modifiers stable area`).toBeLessThanOrEqual(0.15);
      for(const [frame,shape] of [[10,first],[40,later]] as const)
        expect(Math.max(...shape.radii)-Math.min(...shape.radii),`${label} path-modifiers radial detail/${frame}`).toBeGreaterThanOrEqual(8);
      expect(Math.max(...first.radii.map((radius,index)=>Math.abs(radius-later.radii[index]!))),`${label} path-modifiers animated radial profile`).toBeGreaterThanOrEqual(3);
    }
    for (const label of ["native","web"] as const) {
      const pixels=(frame:number)=>frameImages.get(`path-modifiers-extra/${frame}`)![label];
      const red=(frame:number,x:number,y:number)=>pixels(frame)[(y*640+x)*4]!;
      const changed=(first:number,last:number,box:readonly [number,number,number,number])=>{
        let count=0;
        const a=pixels(first),b=pixels(last);
        for(let y=box[1];y<box[3];y++)for(let x=box[0];x<box[2];x++){
          const at=(y*640+x)*4;
          if(Math.max(...[0,1,2].map(channel=>Math.abs(a[at+channel]!-b[at+channel]!)))>16)count++;
        }
        return count;
      };
      expect(changed(0,30,[50,0,230,175]),`${label} path-modifiers pucker/bloat changes star`).toBeGreaterThanOrEqual(1000);
      expect(changed(0,30,[370,10,550,170]),`${label} path-modifiers twist changes cross`).toBeGreaterThanOrEqual(3000);
      const span=(frame:number,from:number,to:number,green:number)=>{
        const ys=[];
        for(let y=225;y<295;y++)for(let x=from;x<to;x++){
          const at=(y*640+x)*4;
          if(pixels(frame)[at+1]!>=green)ys.push(y);
        }
        return Math.max(...ys)-Math.min(...ys)+1;
      };
      expect(span(0,40,280,150),`${label} path-modifiers zigzag before simplify`).toBeGreaterThanOrEqual(25);
      expect(span(59,40,280,150),`${label} path-modifiers simplified line`).toBeLessThanOrEqual(4);
      expect(span(0,470,471,150),`${label} path-modifiers thin stroke outline`).toBeLessThanOrEqual(10);
      expect(span(59,470,471,150),`${label} path-modifiers wide stroke outline`).toBeGreaterThanOrEqual(38);
      expect(red(0,320,190),`${label} path-modifiers stroke ring keeps its hole`).toBe(16);
    }
    for (const label of ["native","web"] as const) {
      const moving = frameImages.get("shutter-sampling/20")![label];
      const stopped = frameImages.get("shutter-sampling/45")![label];
      const control = frameImages.get("shutter-sampling-no-shutter/20")![label];
      const partial = (pixels:Uint8Array) => {
        let count = 0;
        for (let index=0;index<pixels.length;index+=4) if (pixels[index]! > 30 && pixels[index]! < 225) count++;
        return count;
      };
      expect(partial(moving),`${label} shutter-sampling shutter coverage`).toBeGreaterThanOrEqual(2000);
      expect(partial(stopped),`${label} shutter-sampling stationary sharp`).toBeLessThanOrEqual(1000);
      expect(partial(control),`${label} shutter-sampling no-shutter control`).toBeLessThanOrEqual(1000);
      expect(moving,`${label} shutter-sampling must change the moving blade`).not.toEqual(control);
    }
    for (const label of ["native","web"] as const) {
      const moving = frameImages.get("echo-trails/30")![label];
      const stopped = frameImages.get("echo-trails/50")![label];
      const control = frameImages.get("echo-trails-no-echo/30")![label];
      const light = (pixels:Uint8Array,x:number) => pixels[(180*640+x)*4]!;
      expect(light(moving,360),`${label} echo-trails current dot`).toBeGreaterThan(200);
      expect(light(moving,330),`${label} echo-trails first echo`).toBeGreaterThanOrEqual(40);
      expect(light(moving,330),`${label} echo-trails first echo upper bound`).toBeLessThanOrEqual(220);
      expect(light(moving,300),`${label} echo-trails second echo decay`).toBeLessThan(light(moving,330));
      expect(light(moving,200),`${label} echo-trails empty background`).toBeLessThan(30);
      expect(light(stopped,330),`${label} echo-trails stopped trail clears`).toBeLessThan(30);
      expect(light(control,330),`${label} echo-trails no-echo control`).toBeLessThan(30);
      expect(moving,`${label} echo-trails adds past dots`).not.toEqual(control);
    }
    for (const label of ["native","web"] as const) {
      const scoped15 = frameImages.get("time-scope/15")![label];
      const scoped45 = frameImages.get("time-scope/45")![label];
      const plain15 = frameImages.get("time-scope-no-scope/15")![label];
      const plain45 = frameImages.get("time-scope-no-scope/45")![label];
      const light = (pixels:Uint8Array,x:number) => pixels[(180*640+x)*4]!;
      expect(light(scoped15,130),`${label} time-scope offset shortens bar at frame 15`).toBeLessThan(30);
      expect(light(plain15,130),`${label} time-scope no-scope frame 15 control`).toBeGreaterThan(240);
      expect(light(scoped45,290),`${label} time-scope speed lengthens bar at frame 45`).toBeGreaterThan(240);
      expect(light(plain45,290),`${label} time-scope no-scope frame 45 control`).toBeLessThan(30);
      for (const frame of [15,45]) {
        expect(frameImages.get(`time-scope-identity/${frame}`)![label],`${label} time-scope identity scope is layout transparent at frame ${frame}`)
          .toEqual(frameImages.get(`time-scope-no-scope/${frame}`)![label]);
      }
    }
    expect(images.get("alpha"),"opaque circular mask equals circular clip").toEqual(images.get("clip"));
    for (const frame of [0,15,30]) {
      const instanced = frameImages.get(`instances/${frame}`)!;
      const expanded = frameImages.get(`instances-expanded/${frame}`)!;
      expect(instanced.native,`instances/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<instanced.web.length;i++) maximum = Math.max(maximum,Math.abs(instanced.web[i]!-expanded.web[i]!));
      expect(maximum,`instances/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,30,59]) {
      const instanced = frameImages.get(`oklch-color-interpolation-instances/${frame}`)!;
      const expanded = frameImages.get(`oklch-color-interpolation-instances-expanded/${frame}`)!;
      expect(instanced.native,`oklch-color-interpolation-instances/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<instanced.web.length;i++) maximum = Math.max(maximum,Math.abs(instanced.web[i]!-expanded.web[i]!));
      expect(maximum,`oklch-color-interpolation-instances/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const label of ["native","web"] as const) {
      const start = frameImages.get("oklch-color-interpolation-glass/0")![label];
      const middle = frameImages.get("oklch-color-interpolation-glass/30")![label];
      const center = (180*640+320)*4;
      expect(start[center+2]!-start[center+1]!,`${label} Glass starts blue`).toBeGreaterThan(100);
      expect(middle[center+1]!-middle[center+2]!,`${label} Glass reaches teal`).toBeGreaterThan(20);
    }
    for (const frame of [0,1,30]) {
      const optimized = frameImages.get(`lazy-evaluation-mixed/${frame}`)!;
      const expanded = frameImages.get(`lazy-evaluation-mixed-expanded/${frame}`)!;
      expect(optimized.native,`lazy-evaluation-mixed/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-expanded.web[i]!));
      expect(maximum,`lazy-evaluation-mixed/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,1,30]) {
      const optimized = frameImages.get(`lazy-evaluation-nested/${frame}`)!;
      const expanded = frameImages.get(`lazy-evaluation-nested-expanded/${frame}`)!;
      expect(optimized.native,`lazy-evaluation-nested/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-expanded.web[i]!));
      expect(maximum,`lazy-evaluation-nested/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,30]) {
      const optimized = frameImages.get(`lazy-evaluation-certified-hidden/${frame}`)!;
      const expanded = frameImages.get(`lazy-evaluation-certified-hidden-expanded/${frame}`)!;
      expect(optimized.native,`lazy-evaluation-certified-hidden/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-expanded.web[i]!));
      expect(maximum,`lazy-evaluation-certified-hidden/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,1,30]) {
      const optimized = frameImages.get(`lazy-evaluation-instanced/${frame}`)!;
      const expanded = frameImages.get(`lazy-evaluation-expanded/${frame}`)!;
      expect(optimized.native,`lazy-evaluation/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-expanded.web[i]!));
      expect(maximum,`lazy-evaluation/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,8,30]) {
      const optimized = frameImages.get(`lazy-evaluation-affine/${frame}`)!;
      const eager = frameImages.get(`lazy-evaluation-affine-eager/${frame}`)!;
      expect(optimized.native,`lazy-evaluation-affine/${frame} CPU pixel equivalence`).toEqual(eager.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-eager.web[i]!));
      expect(maximum,`lazy-evaluation-affine/${frame} CanvasKit eager equivalence`).toBeLessThanOrEqual(2);
    }
    for (const frame of [0,1,30]) {
      const optimized = frameImages.get(`lazy-evaluation-composed/${frame}`)!;
      const expanded = frameImages.get(`lazy-evaluation-composed-expanded/${frame}`)!;
      expect(optimized.native,`lazy-evaluation-composed/${frame} CPU pixel equivalence`).toEqual(expanded.native);
      let maximum = 0;
      for (let i=0;i<optimized.web.length;i++) maximum = Math.max(maximum,Math.abs(optimized.web[i]!-expanded.web[i]!));
      expect(maximum,`lazy-evaluation-composed/${frame} CanvasKit expanded equivalence`).toBeLessThanOrEqual(2);
    }
    for (const label of ["native","web"] as const) {
      const full = frameImages.get("particle-forces/45")![label];
      const noForces = frameImages.get("particle-forces-none/45")![label];
      let changed = 0;
      for (let i=0;i<full.length;i+=4) {
        if ([0,1,2,3].some(channel=>full[i+channel]!==noForces[i+channel])) changed++;
      }
      expect(changed,`${label} particle-forces forces change pixels`).toBeGreaterThanOrEqual(500);
      expect(frameImages.get("particle-forces-zero-curl/45")![label],`${label} particle-forces zero curl`).toEqual(frameImages.get("particle-forces-drag-only/45")![label]);
      expect(frameImages.get("particle-forces-zero-drag/45")![label],`${label} particle-forces zero drag`).toEqual(frameImages.get("particle-forces-curl-only/45")![label]);
    }
    for (const label of ["native","web"] as const) {
      const wave = frameImages.get("geometry-batch-distance/30")![label];
      const noDelay = frameImages.get("geometry-batch-distance-zero-step/30")![label];
      const center = (index:number) => wave[(204*640+144+100*index)*4]!;
      expect(center(0)).toBe(center(4));
      expect(center(1)).toBe(center(3));
      expect(center(2),`${label} center starts first`).toBeGreaterThan(center(1)+20);
      expect(center(1),`${label} inner starts before outer`).toBeGreaterThan(center(0)+20);
      let changed = 0;
      for (let i=0;i<wave.length;i+=4) if (wave[i]!==noDelay[i]) changed++;
      expect(changed,`${label} distance stagger changes pixels`).toBeGreaterThan(1000);
      expect(frameImages.get("geometry-batch-distance-zero-array/30")![label],`${label} zero array equals step zero`).toEqual(noDelay);
    }
    for (const label of ["native","web"] as const) {
      const moving = frameImages.get("auto-motion-blur/10")![label];
      const stopped = frameImages.get("auto-motion-blur/45")![label];
      const unblurred = frameImages.get("auto-motion-blur-no-auto/10")![label];
      const transitions = (pixels:Uint8Array, indexes:Iterable<number>) => {
        let count = 0;
        for (const index of indexes) if (pixels[index*4]! > 30 && pixels[index*4]! < 225) count++;
        return count;
      };
      const horizontal = (pixels:Uint8Array) => transitions(pixels,Array.from({length:640},(_,x)=>180*640+x));
      const vertical = (pixels:Uint8Array) => transitions(pixels,Array.from({length:360},(_,y)=>y*640+260));
      expect(horizontal(moving),`${label} auto-motion-blur moving edge is blurred`).toBeGreaterThanOrEqual(10);
      expect(horizontal(unblurred),`${label} auto-motion-blur no-auto control`).toBeLessThanOrEqual(2);
      expect(vertical(moving),`${label} auto-motion-blur horizontal direction only`).toBeLessThanOrEqual(4);
      expect(horizontal(stopped),`${label} auto-motion-blur stationary edge is sharp`).toBeLessThanOrEqual(2);
      expect(stopped,`${label} auto-motion-blur stationary equals no blur`).toEqual(frameImages.get("auto-motion-blur-no-auto/45")![label]);
    }
    expect(Math.max(...easingPositions.map(p=>p.back)),"easing-presets back must overshoot").toBeGreaterThan(445);
    expect(easingPositions.find(p=>p.frame===42)!.back).toBe(440);
    expect(new Set(easingPositions.filter(p=>p.frame<30).map(p=>p.steps)).size).toBe(4);
    expect(easingPositions.filter(p=>p.frame===12).map(p=>p.back)[0]).toBe(easingPositions.at(-1)!.back);
    expect(images.get("screen-explicit-srgb")).toEqual(images.get("screen-srgb"));
    expect(images.get("plus-space-independent")).toEqual(images.get("plus-linear"));
    expect(images.get("gradient-default")).toEqual(images.get("gradient-oklab"));
    expect(images.get("gradient-oklch-shorter")).toEqual(images.get("gradient-oklch-decreasing"));
    expect(images.get("gradient-oklch-longer")).toEqual(images.get("gradient-oklch-increasing"));
    expect(images.get("gradient-oklch-shorter")).not.toEqual(images.get("gradient-oklch-longer"));
    const gradientPixels = images.get("gradient")!;
    let partial = 0;
    for (let i=0;i<gradientPixels.length;i+=4) if (gradientPixels[i]! > 20 && gradientPixels[i]! < 235) partial++;
    expect(partial,"soft mask must preserve partial coverage").toBeGreaterThan(400);
    const irisSoft = images.get("subtree-mask-soft")!;
    let irisPartial = 0;
    for (let i=0;i<irisSoft.length;i+=4) if (irisSoft[i+2]! > 20 && irisSoft[i+2]! < 235) irisPartial++;
    expect(irisPartial,"subtree-mask soft-mask negative control").toBeGreaterThanOrEqual(2000);
  } finally { await rm(dir,{ recursive:true,force:true }); }
// This suite renders hundreds of scenes on both backends, including random seeks.
}, 1_200_000);
