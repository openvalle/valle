export const composition = {width:384,height:256,fps:30,duration:2};

export default function UiSubtreeMaterial() {
  const {seconds} = useCurrentTime();
  const {width,height} = useComposition();
  return <Scene style={{width,height,backgroundColor:"#102030"}}>
    <Scene3D key="stage" style={{width,height}}
      camera={{type:"orthographic",height:3,position:[0,0,5],target:[0,0,0]}}>
      <Texture3D>
        <View style={{width:"100%",height:"100%",backgroundColor:"#164a78",padding:20,display:"flex",flexDirection:"column",gap:8}}>
          <Text style={{color:"#ffffff",fontSize:28}}>Live UI</Text>
          <Text style={{color:"#b0d0ff",fontSize:18}}>{seconds<1?"First frame":"Current frame"}</Text>
          <View style={{width:60,height:40,backgroundColor:"#ffba40",borderRadius:8,translate:point(seconds*80,0)}}/>
        </View>
      </Texture3D>
      <Mesh key="panel" geometry={extrude(path("M -1 -0.7 L 1 -0.7 L 1 0.7 L -1 0.7 Z"),{depth:0.04})}
        rotateY={seconds*20} material={{type:"unlit",color:"#ffffff",textures:{baseColor:"input://source"}}}/>
      <Mesh key="front" geometry={extrude(path("M -0.05 -1 L 0.05 -1 L 0.05 1 L -0.05 1 Z"),{depth:0.1})}
        position={[0.3,0,1]} material={{type:"unlit",color:"#dd5533"}}/>
    </Scene3D>
  </Scene>;
}
