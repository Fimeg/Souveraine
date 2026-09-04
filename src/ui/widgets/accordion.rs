//! Collapsible panel that animates between a header-only state and a bordered body.

use std::time::{Duration, Instant};

use chord_macro::chord;
use tuie::prelude::*;

struct Animation {
    start: Instant,
    duration: Duration,
    start_h: u16,
    target_h: u16,
}

impl Animation {
    fn current(&self) -> u16 {
        let t = (self.start.elapsed().as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        let span = self.target_h as f64 - self.start_h as f64;
        (self.start_h as f64 + span * eased).round() as u16
    }

    fn done(&self) -> bool {
        self.start.elapsed() >= self.duration
    }
}

/// Collapsible panel with a clickable header that expands or collapses a body.
pub struct Accordion {
    root: Box<Stack>,
    clipper_id: WidgetId<Pane>,
    inner_id: WidgetId<Pane>,
    title_id: WidgetId<Text>,
    chevron_id: WidgetId<Text>,
    expanded: bool,
    animation: Option<Animation>,
    task: TaskHandle,
}

impl Accordion {
    const HEADER_HEIGHT: u16 = 1;
    const ANIM_DURATION: Duration = Duration::from_millis(150);
    const TICK_INTERVAL: Duration = Duration::from_millis(10);

    fn natural_h(&self) -> u16 {
        self.root
            .get_widget(self.inner_id)
            .map(|i| i.get_rect_size().y)
            .unwrap_or(Self::HEADER_HEIGHT)
            .max(Self::HEADER_HEIGHT)
    }

    fn current_h(&self) -> u16 {
        if let Some(anim) = &self.animation {
            anim.current()
        } else if self.expanded {
            self.root.get_rect_size().y
        } else {
            Self::HEADER_HEIGHT
        }
    }

    fn set_expanded(&mut self, expanded: bool) {
        if self.expanded == expanded {
            return;
        }
        let start_h = self.current_h();
        let target_h = if expanded {
            self.natural_h()
        } else {
            Self::HEADER_HEIGHT
        };
        self.expanded = expanded;

        let chevron = if expanded { "v" } else { ">" };
        if let Some(c) = self.root.get_widget_mut(self.chevron_id) {
            c.set_content(chevron);
        }
        if let Some(clipper) = self.root.get_widget_mut(self.clipper_id) {
            clipper.set_height(Some(start_h));
        }

        self.animation = Some(Animation {
            start: Instant::now(),
            duration: Self::ANIM_DURATION,
            start_h,
            target_h,
        });
        self.task.cancel();
        self.task = tuie::schedule(self.get_id(), Self::TICK_INTERVAL, Accordion::tick);
    }

    fn tick(&mut self) {
        let Some(anim) = self.animation.as_ref() else {
            return;
        };
        let current = anim.current();
        let done = anim.done();
        let expanded = self.expanded;

        if let Some(clipper) = self.root.get_widget_mut(self.clipper_id) {
            if done {
                if expanded {
                    clipper.set_height(None);
                } else {
                    clipper.set_height(Some(Self::HEADER_HEIGHT));
                }
            } else {
                clipper.set_height(Some(current));
            }
        }

        if done {
            self.animation = None;
            self.task = TaskHandle::EMPTY;
        } else {
            self.task = tuie::schedule(self.get_id(), Self::TICK_INTERVAL, Accordion::tick);
        }
    }
}

impl DelegateWidget for Accordion {
    tuie::delegate_widget!(root);

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.next() else {
            return InputResult::Rejected;
        };
        match &event.chord {
            chord!(LeftClick) => {
                let clicked_header = event.pos.y >= 0.0 && event.pos.y < Self::HEADER_HEIGHT as f32;
                if clicked_header {
                    if self.expanded {
                        self.close();
                    } else {
                        self.open();
                    }
                    return InputResult::Handled;
                }
                InputResult::Rejected
            }
            _ => InputResult::Rejected,
        }
    }
}

impl Accordion {
    /// Creates a collapsed [`Accordion`] with the given `title` and `content` body.
    pub fn new(title: &str, content: Box<dyn Widget>) -> Box<Self> {
        let mut inner_id = WidgetId::EMPTY;
        let mut clipper_id = WidgetId::EMPTY;
        let mut title_id = WidgetId::EMPTY;
        let mut chevron_id = WidgetId::EMPTY;

        let root = Stack::new(
            Pane::new()
                .height(Self::HEADER_HEIGHT)
                .id(&mut clipper_id)
                .children([Pane::new()
                    .bordered()
                    .border_style(Style::new().fg(Color::grey256(5)))
                    .id(&mut inner_id)
                    .children([content])]),
        )
        .children([Pane::new()
            .horizontal()
            .flex(1)
            .height(Self::HEADER_HEIGHT)
            .gap(1)
            .style(Style::new().bg(Color::grey256(5)))
            .horizontal_padding(1)
            .y_place(Place::Center)
            .children([
                Text::new()
                    .content(title.to_string())
                    .flex(1)
                    .id(&mut title_id),
                Text::new().content(">").id(&mut chevron_id),
            ])]);

        Box::new(Self {
            root,
            clipper_id,
            inner_id,
            title_id,
            chevron_id,
            expanded: false,
            animation: None,
            task: TaskHandle::EMPTY,
        })
    }

    /// Updates the title text.
    pub fn set_title(&mut self, title: &str) {
        if let Some(t) = self.root.get_widget_mut(self.title_id) {
            t.set_content(title.to_string());
        }
    }

    /// Creates an expanded [`Accordion`] (opens immediately, no animation).
    pub fn new_expanded(title: &str, content: Box<dyn Widget>) -> Box<Self> {
        let mut acc = Self::new(title, content);
        acc.expanded = true;
        if let Some(c) = acc.root.get_widget_mut(acc.chevron_id) {
            c.set_content("v");
        }
        if let Some(clipper) = acc.root.get_widget_mut(acc.clipper_id) {
            clipper.set_height(None);
        }
        acc
    }

    /// Expands the body.
    pub fn open(&mut self) {
        if self.expanded {
            return;
        }
        self.set_expanded(true);
    }

    /// Collapses the body.
    pub fn close(&mut self) {
        if !self.expanded {
            return;
        }
        self.set_expanded(false);
    }
}
