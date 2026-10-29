export const composition = { width: 192, height: 128, duration: 1 };
export default function Dielectric() {
  const time = useCurrentTime();
  const square = path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z");
  return <Scene style={{width:192,height:128,backgroundColor:"#000000"}}>
    <Scene3D key="glass-stage" style={{width:192,height:128}}
      camera={{position:[0,0,5],target:[0,0,0]}}>
      <Mesh key="red" geometry={extrude(square,{depth:0.02})} translateX={-1.6} translateZ={-1} scaleX={1.6} scaleY={3}
        material={{type:"unlit",color:"#ee3020"}}/>
      <Mesh key="blue" geometry={extrude(square,{depth:0.02})} translateX={1.6} translateZ={-1} scaleX={1.6} scaleY={3}
        material={{type:"unlit",color:"#2050ee"}}/>
      <Mesh key="glass" geometry={extrude(square,{depth:0.2,bevel:0.08})} rotateY={25}
        material={{type:"pbr",color:"#ffffff",metallic:0,roughness:0.12,
          transmission:1,thickness:0.2+time.seconds*1.2,attenuationColor:"#ddffff",attenuationDistance:2}}/>
      <Mesh key="foreground" geometry={extrude(square,{depth:0.02})} translateX={-0.2} translateZ={0.8} scaleX={0.06} scaleY={1.3}
        material={{type:"unlit",color:"#ffff00"}}/>
      <AmbientLight intensity={0.2}/>
      <DirectionalLight direction={[0.5,0.5,1]} intensity={0.5}/>
    </Scene3D>
  </Scene>;
}
