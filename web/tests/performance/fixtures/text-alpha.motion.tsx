export const composition = { width: 640, height: 360, fps: 60, duration: 10 };
export default function TextAlpha() { const ctx = useTime();
  return <Scene style={{width:640,height:360}}>
    <View style={{position:"absolute",left:40,top:50,width:560,height:210,opacity:0.5}}>
      <View style={{position:"absolute",left:0,top:0,width:240,height:160,backgroundColor:"#ff6030"}}/>
      <View style={{position:"absolute",left:120,top:40,width:240,height:160,backgroundColor:"#30c0ff"}}/>
      <Text style={{position:"absolute",left:16,top:90,fontSize:36,color:"#fff",opacity:0.5+ctx.progress*0.5}}>VALLE 123</Text>
    </View>
  </Scene>;
}
