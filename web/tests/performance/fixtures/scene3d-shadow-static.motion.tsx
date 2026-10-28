export const composition={width:1920,height:1080,fps:30,duration:2};
const BALL=lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"),{segments:24});
export default function Spatial(){const t=useCurrentTime();return <Scene style={{width:1920,height:1080,backgroundColor:'#101824'}}>
<Scene3D pbr={{shadows:true}} key="stage" style={{width:1920,height:1080}} camera={{position:[Math.sin(t.seconds)*0.3,0,9],target:[0,0,0],fov:45}}>
<AmbientLight intensity={.4}/><DirectionalLight direction={[1,1,2]} intensity={1.5}/>
<Mesh key="ball0" geometry={BALL} position={[-2.55,-0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#ee5b36",roughness:.4,metallic:.2}}/>
<Mesh key="ball1" geometry={BALL} position={[-0.85,-0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#66aaff",roughness:.4,metallic:.2}}/>
<Mesh key="ball2" geometry={BALL} position={[0.85,-0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#ee5b36",roughness:.4,metallic:.2}}/>
<Mesh key="ball3" geometry={BALL} position={[2.55,-0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#66aaff",roughness:.4,metallic:.2}}/>
<Mesh key="ball4" geometry={BALL} position={[-2.55,0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#ee5b36",roughness:.4,metallic:.2}}/>
<Mesh key="ball5" geometry={BALL} position={[-0.85,0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#66aaff",roughness:.4,metallic:.2}}/>
<Mesh key="ball6" geometry={BALL} position={[0.85,0.85,0]} scale={[.6,.6,.6]} material={{type:"pbr",color:"#ee5b36",roughness:.4,metallic:.2}}/>
<Mesh key="receiver" geometry={extrude(path("M -4 -2.4 L 4 -2.4 L 4 2.4 L -4 2.4 Z"),{depth:0.05})} position={[0,0,-1.2]} material={{type:"pbr",color:"#ddd8cc",roughness:0.9,metallic:0}}/>
</Scene3D></Scene>}
