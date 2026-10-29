export const composition = {width: 240, height: 160, duration: 1};
export const role = effect();
export default function UiMaterial() {
  const square = path("M -1 -1 L 1 -1 L 1 1 L -1 1 Z");
  return <Scene style={{width:240,height:160}}>
    <Scene3D key="stage" style={{width:240,height:160}} camera={{position:[0,0,5],target:[0,0,0]}}>
      <Mesh key="screen" geometry={extrude(square,{depth:0.03})} rotateY={25} scaleX={1.7} scaleY={1.2}
        material={{type:"unlit",color:"#ffffff",textures:{baseColor:{src:"input://source",wrapU:"clamp",wrapV:"clamp"}}}}/>
      <Mesh key="behind" geometry={extrude(square,{depth:0.1})} translateZ={-1} scaleX={0.12} scaleY={2}
        material={{type:"unlit",color:"#ff0000"}}/>
      <Mesh key="front" geometry={extrude(square,{depth:0.1})} translateZ={0.8} scaleX={0.05} scaleY={1.3}
        material={{type:"unlit",color:"#ffbb22"}}/>
    </Scene3D>
  </Scene>;
}
