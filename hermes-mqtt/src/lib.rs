#[macro_use]
extern crate failure;
extern crate hermes;
#[cfg(test)]
#[macro_use]
extern crate hermes_test_suite;
#[macro_use]
extern crate log;
#[cfg(test)]
extern crate rand;
extern crate rumqtt;
#[cfg(test)]
extern crate semver;
extern crate serde;
extern crate serde_json;
#[cfg(test)]
extern crate snips_nlu_ontology;
extern crate strum;
#[macro_use]
extern crate strum_macros;
extern crate uuid;

mod topics;

use failure::{ResultExt, SyncFailure};
use hermes::*;
use std::string::ToString;
use std::sync::Arc;
use topics::*;

pub use rumqtt::{MqttOptions, TlsOptions};

struct MqttHandler {
    mqtt_client: rumqtt::MqttClient,
}

impl MqttHandler {
    pub fn publish(&self, topic: &HermesTopic) -> Result<()> {
        let topic = &*topic.as_path();
        debug!("Publishing on MQTT topic '{}'", topic);
        self.mqtt_client
            .publish(topic)
            .and_then(|p| p.send())
            .map_err(SyncFailure::new)?;
        Ok(())
    }

    pub fn publish_payload<P: serde::Serialize>(
        &self,
        topic: &HermesTopic,
        payload: P,
    ) -> Result<()> {
        serde_json::to_vec(&payload).map(|p| {
            let topic = &*topic.as_path();
            debug!(
                "Publishing on MQTT topic '{}', payload: {}",
                topic,
                if p.len() < 2048 {
                    String::from_utf8_lossy(&p).to_string()
                } else {
                    format!(
                        "size = {}, start = {}",
                        p.len(),
                        String::from_utf8_lossy(&p[0..128])
                    )
                }
            );
            trace!("Payload: {}", String::from_utf8_lossy(&p));
            self.mqtt_client
                .publish(topic)
                .map(|m| m.payload(p))
                .and_then(|p| p.send())
                .map_err(SyncFailure::new)
        })??;
        Ok(())
    }

    pub fn publish_binary_payload(&self, topic: &HermesTopic, payload: Vec<u8>) -> Result<()> {
        let topic = &*topic.as_path();
        debug!(
            "Publishing as binary on MQTT topic '{}', with size {}",
            topic,
            payload.len()
        );
        self.mqtt_client
            .publish(topic)
            .map(|m| m.payload(payload))
            .and_then(|p| p.send())
            .map_err(SyncFailure::new)?;

        Ok(())
    }

    pub fn subscribe<F>(&self, topic: &HermesTopic, handler: F) -> Result<()>
    where
        F: Fn() -> () + Send + Sync + 'static,
    {
        let log_level = Self::log_level(topic);
        self.inner_subscribe(topic, move |m| {
            log!(
                log_level,
                "Received a message on MQTT topic '{:?}'",
                m.topic_name
            );
            handler()
        })
    }

    pub fn subscribe_payload<F, P>(&self, topic: &HermesTopic, handler: F) -> Result<()>
    where
        F: Fn(&P) -> () + Send + Sync + 'static,
        P: serde::de::DeserializeOwned,
    {
        let log_level = Self::log_level(topic);
        self.inner_subscribe(topic, move |m| {
            log!(
                log_level,
                "Received a message on MQTT topic '{:?}', payload: {}",
                m.topic_name,
                if m.payload.len() < 2048 {
                    String::from_utf8_lossy(&m.payload).to_string()
                } else {
                    format!(
                        "size = {}, start = {}",
                        m.payload.len(),
                        String::from_utf8_lossy(&m.payload[0..128])
                    )
                }
            );
            trace!("Payload: {}", String::from_utf8_lossy(&m.payload));
            let r = serde_json::from_slice(m.payload.as_slice());
            match r {
                Ok(p) => handler(&p),
                Err(e) => warn!(
                    "Error while decoding object on topic {:?}: {}",
                    m.topic_name, e
                ),
            }
        })
    }

    pub fn subscribe_binary_payload<F>(&self, topic: &HermesTopic, handler: F) -> Result<()>
    where
        F: Fn(&HermesTopic, &[u8]) -> () + Send + Sync + 'static,
    {
        let log_level = Self::log_level(topic);
        self.inner_subscribe(topic, move |m| {
            log!(
                log_level,
                "Received a message on MQTT topic '{:?}', payload: {}",
                m.topic_name,
                if m.payload.len() < 2048 {
                    String::from_utf8_lossy(&m.payload).to_string()
                } else {
                    format!(
                        "size = {}, start = {}",
                        m.payload.len(),
                        String::from_utf8_lossy(&m.payload[0..128])
                    )
                }
            );
            trace!("Payload: {}", String::from_utf8_lossy(&m.payload));
            let topic = HermesTopic::from_path(&m.topic_name);
            if let Some(topic) = topic {
                handler(&topic, &m.payload)
            } else {
                error!("could not parse topic: {:?}", m.topic_name)
            }
        })
    }

    fn inner_subscribe<F>(&self, topic: &HermesTopic, callback: F) -> Result<()>
    where
        F: Fn(&::rumqtt::Publish) -> () + Send + Sync + 'static,
    {
        self.mqtt_client
            .subscribe(topic.to_string(), Box::new(callback))
            .map_err(SyncFailure::new)?
            .send()
            .map_err(SyncFailure::new)?;
        Ok(())
    }

    fn log_level(topic: &HermesTopic) -> ::log::Level {
        match *topic {
            HermesTopic::AudioServer(_, AudioServerCommand::AudioFrame) => ::log::Level::Trace,
            _ => ::log::Level::Debug,
        }
    }
}

pub struct MqttHermesProtocolHandler {
    name: String,
    mqtt_handler: Arc<MqttHandler>,
}

impl MqttHermesProtocolHandler {
    pub fn new(broker_address: &str) -> Result<MqttHermesProtocolHandler> {
        let id = ::uuid::Uuid::new_v4().simple().to_string();
        let client_options = rumqtt::MqttOptions::new(id, broker_address);
        Self::new_with_options(client_options)
    }

    pub fn new_with_options(mut options: rumqtt::MqttOptions) -> Result<MqttHermesProtocolHandler> {
        let name = options.broker_addr.clone();
        options.max_packet_size = 10_000_000;
        let mqtt_client = rumqtt::MqttClient::start(options)
            .map_err(SyncFailure::new)
            .with_context(|_| format_err!("Could not start MQTT client on {}", name))
            .map_err(|e| { error!("Could not start MQTT client: {:?}", e); e})?;

        let mqtt_handler = Arc::new(MqttHandler { mqtt_client });

        Ok(MqttHermesProtocolHandler { name, mqtt_handler })
    }
}

macro_rules! s {
    ($n:ident<$t:ty> $topic:expr; ) => {
        fn $n(&self, handler: Callback<$t>) -> Result<()> {
            self.mqtt_handler.subscribe_payload($topic, move |p| handler.call(p))
        }
    };

    ($n:ident<$t:ty>($($a:ident: $ta:ty),*) $topic:block) => {
        fn $n(&self, $($a: $ta),*, handler: Callback<$t>) -> Result<()> {
            self.mqtt_handler.subscribe_payload($topic, move |p| handler.call(p))
        }
    };

    ($n:ident $topic:expr; ) => {
        fn $n(&self, handler: Callback0) -> Result<()> {
            self.mqtt_handler.subscribe($topic, move || handler.call())
        }
    };
}

macro_rules! s_bin {
    ($n:ident<$t:ty> $topic:block |$rt:ident, $p:ident| $decoder:block) => {
        fn $n(&self, handler: Callback<$t>) -> Result<()> {
            self.mqtt_handler.subscribe_binary_payload($topic, move |$rt, $p| handler.call(&$decoder))
        }
    };

    ($n:ident<$t:ty>($($a:ident: $ta:ty),*) $topic:block |$rt:ident, $p:ident| $decoder:block) => {
        fn $n(&self, $($a: $ta),*, handler: Callback<$t>) -> Result<()> {
            self.mqtt_handler.subscribe_binary_payload($topic, move |$rt, $p| handler.call(&$decoder))
        }
    };
}

macro_rules! p {
    ($n:ident<$t:ty> $topic:expr; ) => {
        fn $n(&self, payload: $t) -> Result<()> {
            self.mqtt_handler.publish_payload($topic, payload)
        }
    };

    ($n:ident<$t:ty>($param1:ident: $t1:ty) $topic:block ) => {
        fn $n(&self, $param1: $t1, payload: $t) -> Result<()> {
            self.mqtt_handler.publish_payload($topic, payload)
        }
    };

    ($n:ident($payload:ident: $t:ty) $topic:block ) => {
        fn $n(&self, $payload: $t) -> Result<()> {
            self.mqtt_handler.publish_payload($topic, $payload)
        }
    };

    ($n:ident $topic:expr; ) => {
        fn $n(&self) -> Result<()> {
            self.mqtt_handler.publish($topic)
        }
    };
}

macro_rules! p_bin {
    ($n:ident($payload:ident: $t:ty) $topic:block $bytes:block ) => {
        fn $n(&self, $payload: $t) -> Result<()> {
            self.mqtt_handler.publish_binary_payload($topic, $bytes)
        }
    };
}

macro_rules! impl_component_facades_for {
    // cannot use s! and p! macros in the impl here because we we would need access to self
    // to get the component... I'm sad...
    ($t:ty) => {
        impl ComponentFacade for $t {
            fn publish_version_request(&self) -> Result<()> {
                self.mqtt_handler.publish(&HermesTopic::Component(
                    None,
                    self.component,
                    ComponentCommand::VersionRequest,
                ))
            }

            fn subscribe_version(&self, handler: Callback<VersionMessage>) -> Result<()> {
                self.mqtt_handler.subscribe_payload(
                    &HermesTopic::Component(None, self.component, ComponentCommand::Version),
                    move |p| handler.call(p),
                )
            }

            fn subscribe_error(&self, handler: Callback<ErrorMessage>) -> Result<()> {
                self.mqtt_handler.subscribe_payload(
                    &HermesTopic::Component(None, self.component, ComponentCommand::Error),
                    move |p| handler.call(p),
                )
            }
        }

        impl ComponentBackendFacade for $t {
            fn subscribe_version_request(&self, handler: Callback0) -> Result<()> {
                self.mqtt_handler.subscribe(
                    &HermesTopic::Component(None, self.component, ComponentCommand::VersionRequest),
                    move || handler.call(),
                )
            }

            fn publish_version(&self, version: VersionMessage) -> Result<()> {
                self.mqtt_handler.publish_payload(
                    &HermesTopic::Component(None, self.component, ComponentCommand::Version),
                    version,
                )
            }

            fn publish_error(&self, error: ErrorMessage) -> Result<()> {
                self.mqtt_handler.publish_payload(
                    &HermesTopic::Component(None, self.component, ComponentCommand::Error),
                    error,
                )
            }
        }
    };
}

macro_rules! impl_toggleable_facades_for {
    // cannot use s! and p! macros in the impl here because we we would need access to self
    // to get the toggle on/off topics... I'm sad...
    ($t:ty) => {
        impl ToggleableFacade for $t {
            fn publish_toggle_on(&self) -> Result<()> {
                self.mqtt_handler.publish(&self.toggle_on_topic)
            }

            fn publish_toggle_off(&self) -> Result<()> {
                self.mqtt_handler.publish(&self.toggle_off_topic)
            }
        }

        impl ToggleableBackendFacade for $t {
            fn subscribe_toggle_on(&self, handler: Callback0) -> Result<()> {
                self.mqtt_handler
                    .subscribe(&self.toggle_on_topic, move || handler.call())
            }

            fn subscribe_toggle_off(&self, handler: Callback0) -> Result<()> {
                self.mqtt_handler
                    .subscribe(&self.toggle_off_topic, move || handler.call())
            }
        }
    };
}

macro_rules! impl_identifiable_toggleable_facades_for {
    ($t:ty) => {
        impl IdentifiableToggleableFacade for $t {
            fn publish_toggle_on(&self, site: SiteMessage) -> Result<()> {
                self.mqtt_handler
                    .publish_payload(&self.toggle_on_topic, site)
            }

            fn publish_toggle_off(&self, site: SiteMessage) -> Result<()> {
                self.mqtt_handler
                    .publish_payload(&self.toggle_off_topic, site)
            }
        }

        impl IdentifiableToggleableBackendFacade for $t {
            fn subscribe_toggle_on(&self, handler: Callback<SiteMessage>) -> Result<()> {
                self.mqtt_handler
                    .subscribe_payload(&self.toggle_on_topic, move |p| handler.call(p))
            }

            fn subscribe_toggle_off(&self, handler: Callback<SiteMessage>) -> Result<()> {
                self.mqtt_handler
                    .subscribe_payload(&self.toggle_off_topic, move |p| handler.call(p))
            }
        }
    };
}

macro_rules! impl_identifiable_component_facades_for {
    ($t:ty) => {
        impl IdentifiableComponentFacade for $t {
            fn publish_version_request(&self, site_id: SiteId) -> Result<()> {
                self.mqtt_handler.publish(&HermesTopic::Component(
                    Some(site_id),
                    self.component,
                    ComponentCommand::VersionRequest,
                ))
            }

            fn subscribe_version(
                &self,
                site_id: SiteId,
                handler: Callback<VersionMessage>,
            ) -> Result<()> {
                self.mqtt_handler.subscribe_payload(
                    &HermesTopic::Component(
                        Some(site_id),
                        self.component,
                        ComponentCommand::Version,
                    ),
                    move |p| handler.call(p),
                )
            }

            fn subscribe_error(
                &self,
                site_id: SiteId,
                handler: Callback<ErrorMessage>,
            ) -> Result<()> {
                self.mqtt_handler.subscribe_payload(
                    &HermesTopic::Component(Some(site_id), self.component, ComponentCommand::Error),
                    move |p| handler.call(p),
                )
            }
        }

        impl IdentifiableComponentBackendFacade for $t {
            fn subscribe_version_request(&self, site_id: SiteId, handler: Callback0) -> Result<()> {
                self.mqtt_handler.subscribe(
                    &HermesTopic::Component(
                        Some(site_id),
                        self.component,
                        ComponentCommand::VersionRequest,
                    ),
                    move || handler.call(),
                )
            }

            fn publish_version(&self, site_id: SiteId, version: VersionMessage) -> Result<()> {
                self.mqtt_handler.publish_payload(
                    &HermesTopic::Component(
                        Some(site_id),
                        self.component,
                        ComponentCommand::Version,
                    ),
                    version,
                )
            }

            fn publish_error(&self, site_id: SiteId, error: ErrorMessage) -> Result<()> {
                self.mqtt_handler.publish_payload(
                    &HermesTopic::Component(Some(site_id), self.component, ComponentCommand::Error),
                    error,
                )
            }
        }
    };
}

struct MqttComponentFacade {
    component: Component,
    mqtt_handler: Arc<MqttHandler>,
}

impl_component_facades_for!(MqttComponentFacade);
impl_identifiable_component_facades_for!(MqttComponentFacade);

struct MqttToggleableFacade {
    toggle_on_topic: HermesTopic,
    toggle_off_topic: HermesTopic,
    mqtt_handler: Arc<MqttHandler>,
}

impl_identifiable_toggleable_facades_for!(MqttToggleableFacade);

struct MqttToggleableComponentFacade {
    component: Component,
    toggle_on_topic: HermesTopic,
    toggle_off_topic: HermesTopic,
    mqtt_handler: Arc<MqttHandler>,
}

impl_component_facades_for!(MqttToggleableComponentFacade);
impl_toggleable_facades_for!(MqttToggleableComponentFacade);
impl_identifiable_component_facades_for!(MqttToggleableComponentFacade);
impl_identifiable_toggleable_facades_for!(MqttToggleableComponentFacade);

impl HotwordFacade for MqttToggleableComponentFacade {
    s!(subscribe_detected<HotwordDetectedMessage>(site_id: String) { &HermesTopic::Hotword(Some(site_id), HotwordCommand::Detected) });
    s!(subscribe_all_detected<HotwordDetectedMessage> &HermesTopic::Hotword(Some("+".into()), HotwordCommand::Detected););
}

impl HotwordBackendFacade for MqttToggleableComponentFacade {
    p!(publish_detected<HotwordDetectedMessage>(site_id: String) { &HermesTopic::Hotword(Some(site_id), HotwordCommand::Detected) });
}

impl SoundFeedbackFacade for MqttToggleableFacade {}

impl SoundFeedbackBackendFacade for MqttToggleableFacade {}

impl AsrFacade for MqttToggleableComponentFacade {
    p!(publish_start_listening<SiteMessage> &HermesTopic::Asr(AsrCommand::StartListening););
    p!(publish_stop_listening<SiteMessage> &HermesTopic::Asr(AsrCommand::StopListening););
    p!(publish_reload &HermesTopic::Asr(AsrCommand::Reload););
    p!(publish_injection_request<InjectionRequest> &HermesTopic::Asr(AsrCommand::Inject););
    p!(publish_injection_status_request &HermesTopic::Asr(AsrCommand::InjectStatusRequest););
    s!(subscribe_text_captured<TextCapturedMessage> &HermesTopic::Asr(AsrCommand::TextCaptured););
    s!(subscribe_partial_text_captured<TextCapturedMessage> &HermesTopic::Asr(AsrCommand::PartialTextCaptured););
    s!(subscribe_injection_status<InjectionStatus> &HermesTopic::Asr(AsrCommand::InjectStatus););
}

impl AsrBackendFacade for MqttToggleableComponentFacade {
    s!(subscribe_start_listening<SiteMessage> &HermesTopic::Asr(AsrCommand::StartListening););
    s!(subscribe_stop_listening<SiteMessage> &HermesTopic::Asr(AsrCommand::StopListening););
    s!(subscribe_reload &HermesTopic::Asr(AsrCommand::Reload););
    s!(subscribe_injection_request<InjectionRequest> &HermesTopic::Asr(AsrCommand::Inject););
    s!(subscribe_injection_status_request &HermesTopic::Asr(AsrCommand::InjectStatusRequest););
    p!(publish_text_captured<TextCapturedMessage> &HermesTopic::Asr(AsrCommand::TextCaptured););
    p!(publish_partial_text_captured<TextCapturedMessage> &HermesTopic::Asr(AsrCommand::PartialTextCaptured););
    p!(publish_injection_status<InjectionStatus> &HermesTopic::Asr(AsrCommand::InjectStatus););
}

impl TtsFacade for MqttComponentFacade {
    p!(publish_say<SayMessage> &HermesTopic::Tts(TtsCommand::Say););
    s!(subscribe_say_finished<SayFinishedMessage> &HermesTopic::Tts(TtsCommand::SayFinished););
}

impl TtsBackendFacade for MqttComponentFacade {
    s!(subscribe_say<SayMessage> &HermesTopic::Tts(TtsCommand::Say););
    p!(publish_say_finished<SayFinishedMessage> &HermesTopic::Tts(TtsCommand::SayFinished););
}

impl NluFacade for MqttComponentFacade {
    p!(publish_query<NluQueryMessage> &HermesTopic::Nlu(NluCommand::Query););
    p!(publish_partial_query<NluSlotQueryMessage> &HermesTopic::Nlu(NluCommand::PartialQuery););
    s!(subscribe_slot_parsed<NluSlotMessage> &HermesTopic::Nlu(NluCommand::SlotParsed););
    s!(subscribe_intent_parsed<NluIntentMessage> &HermesTopic::Nlu(NluCommand::IntentParsed););
    s!(subscribe_intent_not_recognized<NluIntentNotRecognizedMessage> &HermesTopic::Nlu(NluCommand::IntentNotRecognized););
}

impl NluBackendFacade for MqttComponentFacade {
    s!(subscribe_query<NluQueryMessage> &HermesTopic::Nlu(NluCommand::Query););
    s!(subscribe_partial_query<NluSlotQueryMessage> &HermesTopic::Nlu(NluCommand::PartialQuery););
    p!(publish_slot_parsed<NluSlotMessage> &HermesTopic::Nlu(NluCommand::SlotParsed););
    p!(publish_intent_parsed<NluIntentMessage> &HermesTopic::Nlu(NluCommand::IntentParsed););
    p!(publish_intent_not_recognized<NluIntentNotRecognizedMessage> &HermesTopic::Nlu(NluCommand::IntentNotRecognized););
}

impl AudioServerFacade for MqttToggleableComponentFacade {
    s_bin!(subscribe_audio_frame<AudioFrameMessage>(site_id: SiteId) { &HermesTopic::AudioServer(Some(site_id), AudioServerCommand::AudioFrame) }
            |topic, bytes| {
                if let HermesTopic::AudioServer(Some(ref site_id), AudioServerCommand::AudioFrame) = *topic {
                    AudioFrameMessage { site_id: site_id.to_owned(), wav_frame: bytes.into() }
                } else {
                    unreachable!()
                }
            });
    p_bin!(publish_play_bytes(bytes: PlayBytesMessage)
        { &HermesTopic::AudioServer(Some(bytes.site_id), AudioServerCommand::PlayBytes(bytes.id)) }
        { bytes.wav_bytes });
    s!(subscribe_play_finished<PlayFinishedMessage>(site_id: SiteId) { &HermesTopic::AudioServer(Some(site_id), AudioServerCommand::PlayFinished) });
    s!(subscribe_all_play_finished<PlayFinishedMessage> &HermesTopic::AudioServer(Some("+".into()), AudioServerCommand::PlayFinished););
}

impl AudioServerBackendFacade for MqttToggleableComponentFacade {
    p_bin!(publish_audio_frame(frame: AudioFrameMessage)
        { &HermesTopic::AudioServer(Some(frame.site_id), AudioServerCommand::AudioFrame) }
        { frame.wav_frame });
    s_bin!(subscribe_all_play_bytes<PlayBytesMessage> { &HermesTopic::AudioServer(Some("+".into()), AudioServerCommand::PlayBytes("#".into())) }
            |topic, bytes| {
                if let HermesTopic::AudioServer(Some(ref site_id), AudioServerCommand::PlayBytes(ref request_id)) = *topic {
                    PlayBytesMessage { site_id: site_id.to_owned(), id: request_id.to_owned(), wav_bytes: bytes.into() }
                } else {
                    unreachable!()
                }
            });
    s_bin!(subscribe_play_bytes<PlayBytesMessage>(site_id: SiteId) { &HermesTopic::AudioServer(Some(site_id), AudioServerCommand::PlayBytes("#".into())) }
            |topic, bytes| {
                if let HermesTopic::AudioServer(Some(ref site_id), AudioServerCommand::PlayBytes(ref request_id)) = *topic {
                    PlayBytesMessage { site_id: site_id.to_owned(), id: request_id.to_owned(), wav_bytes: bytes.into() }
                } else {
                    unreachable!()
                }
            });
    p!(publish_play_finished(message: PlayFinishedMessage) { &HermesTopic::AudioServer(Some(message.site_id.clone()), AudioServerCommand::PlayFinished) });
}

impl DialogueFacade for MqttToggleableComponentFacade {
    s!(subscribe_session_queued<SessionQueuedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionQueued););
    s!(subscribe_session_started<SessionStartedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionStarted););
    s!(subscribe_intent<IntentMessage>(intent_name: String) { &HermesTopic::Intent(intent_name) });
    s!(subscribe_intents<IntentMessage> &HermesTopic::Intent("#".into()););
    s!(subscribe_intent_not_recognized<IntentNotRecognizedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::IntentNotRecognized););
    s!(subscribe_session_ended<SessionEndedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionEnded););
    p!(publish_start_session<StartSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::StartSession););
    p!(publish_continue_session<ContinueSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::ContinueSession););
    p!(publish_end_session<EndSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::EndSession););
}

impl DialogueBackendFacade for MqttToggleableComponentFacade {
    p!(publish_session_queued<SessionQueuedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionQueued););
    p!(publish_session_started<SessionStartedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionStarted););
    p!(publish_intent(intent: IntentMessage) {&HermesTopic::Intent(intent.intent.intent_name.clone())});
    p!(publish_intent_not_recognized<IntentNotRecognizedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::IntentNotRecognized););
    p!(publish_session_ended<SessionEndedMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::SessionEnded););
    s!(subscribe_start_session<StartSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::StartSession););
    s!(subscribe_continue_session<ContinueSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::ContinueSession););
    s!(subscribe_end_session<EndSessionMessage> &HermesTopic::DialogueManager(DialogueManagerCommand::EndSession););
}

impl MqttHermesProtocolHandler {
    fn hotword_component(&self) -> Box<MqttToggleableComponentFacade> {
        Box::new(MqttToggleableComponentFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            component: Component::Hotword,
            toggle_on_topic: HermesTopic::Hotword(None, HotwordCommand::ToggleOn),
            toggle_off_topic: HermesTopic::Hotword(None, HotwordCommand::ToggleOff),
        })
    }

    fn sound_toggleable(&self) -> Box<MqttToggleableFacade> {
        Box::new(MqttToggleableFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            toggle_on_topic: HermesTopic::Feedback(FeedbackCommand::Sound(SoundCommand::ToggleOn)),
            toggle_off_topic: HermesTopic::Feedback(FeedbackCommand::Sound(
                SoundCommand::ToggleOff,
            )),
        })
    }

    fn asr_component(&self) -> Box<MqttToggleableComponentFacade> {
        Box::new(MqttToggleableComponentFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            component: Component::Asr,
            toggle_on_topic: HermesTopic::Asr(AsrCommand::ToggleOn),
            toggle_off_topic: HermesTopic::Asr(AsrCommand::ToggleOff),
        })
    }

    fn dialogue_component(&self) -> Box<MqttToggleableComponentFacade> {
        Box::new(MqttToggleableComponentFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            component: Component::DialogueManager,
            toggle_on_topic: HermesTopic::DialogueManager(DialogueManagerCommand::ToggleOn),
            toggle_off_topic: HermesTopic::DialogueManager(DialogueManagerCommand::ToggleOff),
        })
    }

    fn audio_server_component(&self) -> Box<MqttToggleableComponentFacade> {
        Box::new(MqttToggleableComponentFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            component: Component::AudioServer,
            toggle_on_topic: HermesTopic::AudioServer(None, AudioServerCommand::ToggleOn),
            toggle_off_topic: HermesTopic::AudioServer(None, AudioServerCommand::ToggleOff),
        })
    }

    fn component(&self, component: Component) -> Box<MqttComponentFacade> {
        Box::new(MqttComponentFacade {
            mqtt_handler: Arc::clone(&self.mqtt_handler),
            component,
        })
    }
}

impl HermesProtocolHandler for MqttHermesProtocolHandler {
    fn hotword(&self) -> Box<HotwordFacade> {
        self.hotword_component()
    }

    fn sound_feedback(&self) -> Box<SoundFeedbackFacade> {
        self.sound_toggleable()
    }

    fn asr(&self) -> Box<AsrFacade> {
        self.asr_component()
    }

    fn tts(&self) -> Box<TtsFacade> {
        self.component(Component::Tts)
    }

    fn nlu(&self) -> Box<NluFacade> {
        self.component(Component::Nlu)
    }

    fn audio_server(&self) -> Box<AudioServerFacade> {
        self.audio_server_component()
    }

    fn dialogue(&self) -> Box<DialogueFacade> {
        self.dialogue_component()
    }

    fn hotword_backend(&self) -> Box<HotwordBackendFacade> {
        self.hotword_component()
    }

    fn sound_feedback_backend(&self) -> Box<SoundFeedbackBackendFacade> {
        self.sound_toggleable()
    }

    fn asr_backend(&self) -> Box<AsrBackendFacade> {
        self.asr_component()
    }

    fn tts_backend(&self) -> Box<TtsBackendFacade> {
        self.component(Component::Tts)
    }

    fn nlu_backend(&self) -> Box<NluBackendFacade> {
        self.component(Component::Nlu)
    }

    fn audio_server_backend(&self) -> Box<AudioServerBackendFacade> {
        self.audio_server_component()
    }

    fn dialogue_backend(&self) -> Box<DialogueBackendFacade> {
        self.dialogue_component()
    }
}

impl std::fmt::Display for MqttHermesProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} (MQTT)", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::rc::Rc;
    use std::thread::sleep;
    use std::time::Duration;

    struct ServerHolder {
        server: ::std::process::Child,
    }

    struct HandlerHolder {
        handler: MqttHermesProtocolHandler,
        // this code is not dead, we need this as there is a drop on server holder that will kill
        // the child process
        #[allow(dead_code)]
        server: Rc<ServerHolder>,
    }

    impl std::ops::Deref for HandlerHolder {
        type Target = MqttHermesProtocolHandler;
        fn deref(&self) -> &MqttHermesProtocolHandler {
            &self.handler
        }
    }

    impl Drop for ServerHolder {
        fn drop(&mut self) {
            self.server.kill().unwrap();
        }
    }

    fn create_handlers() -> (HandlerHolder, HandlerHolder) {
        // get a random free port form the OS
        let port = {
            TcpListener::bind("localhost:0").unwrap().local_addr().unwrap().port()
        };

        // /usr/sbin is not in path on non login session on raspbian and it is where mosquitto is
        ::std::env::set_var("PATH", format!("{}:/usr/sbin",::std::env::var("PATH").unwrap()));

        let server = Rc::new(ServerHolder {
            server: Command::new("mosquitto")
                .arg("-p")
                .arg(format!("{}", port))
                .arg("-v")
                .spawn()
                .expect("could not start mosquitto"),
        });

        let server_address = format!("localhost:{}", port);

        // wait 'till mosquitto is accessible.
        let server_is_live = || {
            for _ in 0..10 {
                if TcpStream::connect(&server_address).is_ok() {
                    return true
                } else {
                    sleep(Duration::from_millis(50));
                }
            };
            false
        };
        assert!(server_is_live(), format!("can't connect to mosquitto server {}", &server_address));

        let handler1 = HandlerHolder {
            handler: MqttHermesProtocolHandler::new(&server_address)
                .expect("could not create first client"),
            server: Rc::clone(&server),
        };

        let handler2 = HandlerHolder {
            handler: MqttHermesProtocolHandler::new(&server_address)
                .expect("could not create second client"),
            server,
        };

        (handler1, handler2)
    }

    // sleep 10ms between registering the callback and sending the message to be "sure" the event
    // arrive in the right order to the mosquitto server
    test_suite!(WAIT_DURATION = 10);
}
