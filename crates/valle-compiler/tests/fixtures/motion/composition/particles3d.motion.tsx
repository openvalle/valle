export const composition = { width:384, height:256, fps:30, duration:3 };
const BALL = lathe(path("M 0 -0.7 C 0.39 -0.7 0.7 -0.39 0.7 0 C 0.7 0.39 0.39 0.7 0 0.7"), {segments:24});
export default function Main() {
  const {seconds} = useCurrentTime();
  return <Scene style={{backgroundColor:"#0c1525"}}>
    <Scene3D key="space" style={{width:384,height:256}}
      camera={{position:[0.5,0.3,5],target:[0,0,0],orbitYaw:seconds*12}}
      pbr={{toneMapping:"aces"}}>
      <Mesh key="core" geometry={BALL} rotateY={seconds*30}
        material={{type:"pbr",color:"#5787b3",metallic:0.5,roughness:0.3}} />
      <Group3D key="flow" rotateZ={seconds*8}>
        <Particles3D key="sparks" positions={particles3d(seconds, {
          seed:17, count:72, emitter:{min:[-1.4,-0.8,-1],max:[1.4,0.8,1]},
          birth:{interval:0}, lifetime:4, loop:true,
          velocity:{x:[-0.1,0.1],y:[0.1,0.3],z:[-0.15,0.15]},
          gravity:[0,-0.12,0], drag:0.3
        })} sizes={[0.12,0.03]} fills={["#ffc57a","#629bff"]} opacities={[0.9,0.15]} />
      </Group3D>
      <AmbientLight intensity={0.3}/>
      <DirectionalLight direction={[1,1,2]} intensity={2}/>
    </Scene3D>
  </Scene>;
}
