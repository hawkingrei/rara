#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValueSource {
    ProviderState,
    LegacyGlobal,
    BuiltInDefault,
    Unset,
}

impl ConfigValueSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderState => "provider_state",
            Self::LegacyGlobal => "legacy_global",
            Self::BuiltInDefault => "built_in_default",
            Self::Unset => "unset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedProviderValue<'a> {
    pub value: Option<&'a str>,
    pub source: ConfigValueSource,
}

impl<'a> ResolvedProviderValue<'a> {
    pub fn display_or<'b>(self, fallback: &'b str) -> &'b str
    where
        'a: 'b,
    {
        self.value.unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveProviderSurface<'a> {
    pub provider: &'a str,
    pub model: ResolvedProviderValue<'a>,
    pub base_url: ResolvedProviderValue<'a>,
    pub revision: ResolvedProviderValue<'a>,
    pub reasoning_effort: ResolvedProviderValue<'a>,
    pub reasoning_summary: ResolvedProviderValue<'a>,
    pub api_key: ResolvedProviderValue<'a>,
}
