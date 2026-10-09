use slsk::proto::server::{ServerRequest, ServerResponse};
use slsk::proto::types::Recommendation;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimilarUser {
    pub username: String,
    pub weight: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemDetails {
    pub item: String,
    pub recommendations: Vec<Recommendation>,
    pub users: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Discovery {
    pub recommended: Vec<Recommendation>,
    pub unrecommended: Vec<Recommendation>,
    pub global: Vec<Recommendation>,
    pub similar: Vec<SimilarUser>,
    pub item: Option<ItemDetails>,
}

/// Interests and the recommendations the server builds from them.
#[derive(Default)]
pub struct Discover {
    likes: Vec<String>,
    dislikes: Vec<String>,
    discovery: Discovery,
}

impl Discover {
    pub fn set_interests(&mut self, likes: Vec<String>, dislikes: Vec<String>) {
        self.likes = likes;
        self.dislikes = dislikes;
    }

    /// The server forgets interests between sessions, so the saved ones are sent at login.
    pub fn announce(&self) -> Vec<ServerRequest> {
        let likes = self.likes.iter().cloned().map(ServerRequest::AddThingILike);
        let dislikes = self
            .dislikes
            .iter()
            .cloned()
            .map(ServerRequest::AddThingIHate);
        likes.chain(dislikes).collect()
    }

    pub fn set_interest(&mut self, item: String, like: bool, add: bool) -> Vec<ServerRequest> {
        let list = if like {
            &mut self.likes
        } else {
            &mut self.dislikes
        };
        list.retain(|existing| *existing != item);
        if add {
            list.push(item.clone());
        }
        let change = match (like, add) {
            (true, true) => ServerRequest::AddThingILike(item),
            (true, false) => ServerRequest::RemoveThingILike(item),
            (false, true) => ServerRequest::AddThingIHate(item),
            (false, false) => ServerRequest::RemoveThingIHate(item),
        };
        let mut requests = vec![change];
        requests.extend(self.refresh());
        requests
    }

    pub fn refresh(&self) -> Vec<ServerRequest> {
        vec![
            ServerRequest::Recommendations,
            ServerRequest::GlobalRecommendations,
            ServerRequest::SimilarUsers,
        ]
    }

    pub fn open_item(&mut self, item: String) -> Vec<ServerRequest> {
        self.discovery.item = Some(ItemDetails {
            item: item.clone(),
            ..Default::default()
        });
        vec![
            ServerRequest::ItemRecommendations(item.clone()),
            ServerRequest::ItemSimilarUsers(item),
        ]
    }

    /// Folds a server reply in, returning the new state when it changed.
    pub fn apply(&mut self, message: &ServerResponse) -> Option<Discovery> {
        let before = self.discovery.clone();
        match message {
            ServerResponse::Recommendations { likes, dislikes } => {
                self.discovery.recommended = likes.clone();
                self.discovery.unrecommended = dislikes.clone();
            }
            ServerResponse::GlobalRecommendations { likes, .. } => {
                self.discovery.global = likes.clone();
            }
            ServerResponse::SimilarUsers(users) => {
                self.discovery.similar = users
                    .iter()
                    .map(|(username, rating)| SimilarUser {
                        username: username.clone(),
                        weight: *rating,
                    })
                    .collect();
            }
            ServerResponse::ItemRecommendations {
                item,
                recommendations,
            } => {
                if let Some(details) = self.discovery.item.as_mut().filter(|d| d.item == *item) {
                    details.recommendations = recommendations.clone();
                }
            }
            ServerResponse::ItemSimilarUsers { item, usernames } => {
                if let Some(details) = self.discovery.item.as_mut().filter(|d| d.item == *item) {
                    details.users = usernames.clone();
                }
            }
            _ => return None,
        }
        (self.discovery != before).then(|| self.discovery.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_in_replies_for_the_open_item_only() {
        let mut discover = Discover::default();
        discover.open_item("ambient".into());
        let recs = vec![Recommendation {
            item: "drone".into(),
            score: 5,
        }];
        let next = discover.apply(&ServerResponse::ItemRecommendations {
            item: "ambient".into(),
            recommendations: recs.clone(),
        });
        assert_eq!(next.unwrap().item.unwrap().recommendations, recs);
        assert!(
            discover
                .apply(&ServerResponse::ItemRecommendations {
                    item: "jazz".into(),
                    recommendations: recs,
                })
                .is_none()
        );
    }

    #[test]
    fn sends_interest_changes_with_a_refresh() {
        let mut discover = Discover::default();
        let requests = discover.set_interest("idm".into(), true, true);
        assert_eq!(requests[0], ServerRequest::AddThingILike("idm".into()));
        assert_eq!(requests.len(), 4);
        assert_eq!(
            discover.announce(),
            vec![ServerRequest::AddThingILike("idm".into())]
        );
    }
}
