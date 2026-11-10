//! Video resources bound by the host and sampled by a scene.

use super::Scene;

use crate::front::{Holds, Inputs};
use crate::image::{Image, Picture};
use crate::style::Style;
use crate::template::{Error, Kind, Prop, Template, V};
use crate::time::Tick;

/// A video the host decodes, for a video resource.
pub trait Video {
    /// The size of its frames, in px.
    fn size(&self) -> [u32; 2];
    /// The frame it shows `at` into it: the last that starts at or before then, or the first,
    /// before it starts.
    fn frame(&mut self, at: Tick) -> Result<Image, String>;
}

/// A video resource's decoder, the image its frames go into, and the time of the one there.
pub(super) struct Shown {
    pub(super) video: Box<dyn Video>,
    pub(super) image: usize,
    pub(super) at: Option<Tick>,
}

impl Scene {
    /// The file of each video resource, by its name: the one bound to it, else its default.
    pub fn video_files(&self, inputs: &Inputs) -> Vec<(String, String)> {
        (self.front.resources.iter())
            .filter(|s| s.kind == Holds::Video)
            .filter_map(|s| {
                let files = inputs.resources.get(&s.name).unwrap_or(&s.default);
                Some((s.name.clone(), files.first()?.clone()))
            })
            .collect()
    }

    /// Gives video resource `name` its decoder.
    pub fn set_video(&mut self, name: &str, video: Box<dyn Video>) -> Result<(), Error> {
        let slot = (self.front.resources.iter())
            .position(|s| s.name == name && s.kind == Holds::Video)
            .ok_or_else(|| format!("the scene declares no video resource {name}"))?;
        let size = video.size();
        let pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        self.images.push(Image::new(size, pixels.into()));
        let image = self.images.len() - 1;
        self.pictures[slot] = Picture::Image {
            image: image as u32,
            size,
        };
        self.videos[slot] = Some(Shown {
            video,
            image,
            at: None,
        });
        Ok(())
    }

    /// The frame of each video a shown `<Video>` shows, decoded into its image. A video
    /// resource shows one time in a frame.
    pub(super) fn show_videos(
        &mut self,
        template: &Template,
        values: &[V],
        styles: &[Style],
    ) -> Result<(), Error> {
        let mut asked: Vec<Option<Tick>> = vec![None; self.videos.len()];
        // `display: none` removes a whole subtree, including a timeline clip outside its span.
        // A child's computed display alone does not say whether its ancestors are shown.
        let mut hidden = vec![false; template.nodes.len()];
        for (i, node) in template.nodes.iter().enumerate() {
            hidden[i] |= styles[i].layout.display == taffy::Display::None;
            if hidden[i] {
                for &child in &node.children {
                    hidden[child as usize] = true;
                }
                continue;
            }
            let Kind::Image {
                resource,
                video: Some(time),
            } = &node.kind
            else {
                continue;
            };
            let s = &styles[i];
            if !s.visible {
                continue;
            }
            let r = *resource as usize;
            let name = &self.front.resources[r].name;
            let seconds = match time {
                Prop::Const(V::Num(x)) => *x,
                Prop::Expr(e) => match &values[*e as usize] {
                    V::Num(x) => *x,
                    v => {
                        return Err(format!(
                            "{}: <Video> time is a number, not {v:?}",
                            node.place()
                        ));
                    }
                },
                Prop::Const(v) => {
                    return Err(format!(
                        "{}: <Video> time is a number, not {v:?}",
                        node.place()
                    ));
                }
            };
            let at = Tick::from_seconds(seconds)
                .map_err(|e| format!("{}: <Video> time: {e}", node.place()))?;
            match asked[r] {
                Some(other) if other != at => {
                    return Err(format!(
                        "{}: <Video> resource://{name} shows {} s here and {} s elsewhere in the \
                         frame: a video resource shows one time a frame, so declare another \
                         resource of the same file",
                        node.place(),
                        at.seconds_text(),
                        other.seconds_text()
                    ));
                }
                _ => asked[r] = Some(at),
            }
        }
        for (r, at) in asked.into_iter().enumerate() {
            let Some(at) = at else { continue };
            let name = &self.front.resources[r].name;
            let shown = self.videos[r]
                .as_mut()
                .ok_or_else(|| format!("video resource {name}: the host gave no decoder for it"))?;
            if shown.at == Some(at) {
                continue;
            }
            let frame =
                (shown.video.frame(at)).map_err(|e| format!("video resource {name}: {e}"))?;
            if frame.size != self.images[shown.image].size {
                return Err(format!("video resource {name}: a frame of another size"));
            }
            self.images[shown.image] = frame;
            shown.at = Some(at);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    use crate::testing::fonts;

    use crate::scene::*;

    /// Runs `f` on a thread with the stack scene code needs.
    fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .expect("a thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    fn load(source: &'static str) -> Result<Scene, Error> {
        Scene::load(
            "s.motion.tsx",
            &|path| (path == "s.motion.tsx").then(|| source.as_bytes().to_vec()),
            &Inputs::default(),
            fonts(),
        )
    }

    #[test]
    fn hidden_ancestors_do_not_decode_video_but_visible_descendants_can() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Count(Rc<RefCell<Vec<Tick>>>);
        impl Video for Count {
            fn size(&self) -> [u32; 2] {
                [2, 2]
            }
            fn frame(&mut self, at: Tick) -> Result<Image, String> {
                self.0.borrow_mut().push(at);
                Ok(Image::new([2, 2], vec![255; 16].into()))
            }
        }
        big_stack(|| {
            let mut scene = load(indoc! {"
                export const composition = { width: 32, height: 32, fps: 30, duration: 2 };
                export const controls = { resources: { clip: resource({kind:'video', default:'./clip.mp4'}) } };
                export default function Scene() {
                  const {seconds} = useCurrentTime();
                  return <View style={{display: seconds < 1 ? 'none' : 'flex'}}>
                    <View style={{visibility:'hidden'}}>
                      <Video src='resource://clip' style={{visibility:'visible', width:32, height:32}} />
                    </View>
                  </View>;
                }
            "}).unwrap();
            let calls = Rc::new(RefCell::new(Vec::new()));
            scene
                .set_video("clip", Box::new(Count(calls.clone())))
                .unwrap();
            scene.frame(Tick::ZERO).unwrap();
            assert!(calls.borrow().is_empty());
            let later = Tick::from_seconds(1.0).unwrap();
            scene.frame(later).unwrap();
            assert_eq!(&*calls.borrow(), &[later]);
            scene.frame(Tick::ZERO).unwrap();
            assert_eq!(calls.borrow().len(), 1);
            scene.frame(later).unwrap();
            assert_eq!(calls.borrow().len(), 1);
        });
    }

    #[test]
    fn changing_a_video_sources_natural_size_invalidates_layout() {
        struct Solid([u32; 2]);
        impl Video for Solid {
            fn size(&self) -> [u32; 2] {
                self.0
            }
            fn frame(&mut self, _: Tick) -> Result<Image, String> {
                Ok(Image::new(
                    self.0,
                    vec![255; (self.0[0] * self.0[1] * 4) as usize].into(),
                ))
            }
        }
        big_stack(|| {
            let mut scene = load(indoc! {"
            export const composition = {width: 200, height: 100, fps: 30, duration: 2};
            export const controls = {resources: {clip: resource({kind:'video', default:'./clip.mov'})}};
            export default function Scene() { return <Video src='resource://clip' style={{width:40}} />; }
        "}).unwrap();
            scene.set_video("clip", Box::new(Solid([4, 2]))).unwrap();
            let first = scene.frame(Tick(0)).unwrap();
            let next = scene.frame(Tick::parse_seconds("0.5").unwrap()).unwrap();
            assert!(Rc::ptr_eq(&first.layout, &next.layout));
            assert_eq!(next.layout.boxes[0].border.height, 20.0);
            scene.set_video("clip", Box::new(Solid([2, 4]))).unwrap();
            let changed = scene.frame(Tick::parse_seconds("1").unwrap()).unwrap();
            assert!(!Rc::ptr_eq(&next.layout, &changed.layout));
            assert_eq!(changed.layout.boxes[0].border.height, 80.0);
        });
    }
}
