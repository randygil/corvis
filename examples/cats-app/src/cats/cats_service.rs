use std::sync::RwLock;

use corvis::prelude::*;

use super::dto::{CreateCatDto, ListCatsQuery, UpdateCatDto};
use super::entities::{Cat, CatCreatedEvent};

#[injectable]
pub struct CatsService {
    #[default]
    cats: RwLock<Vec<Cat>>,
    event_emitter: Arc<EventEmitter>,
    #[init(Logger::new("CatsService"))]
    logger: Logger,
}

impl CatsService {
    pub fn create(&self, dto: CreateCatDto, owner_id: u32) -> Cat {
        let mut cats = self.cats.write().unwrap();
        let id = cats.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let cat = Cat { id, name: dto.name, age: dto.age, breed: dto.breed, owner_id };
        cats.push(cat.clone());
        self.event_emitter.emit("cat.created", CatCreatedEvent { cat: cat.clone() });
        cat
    }

    pub fn find_all(&self, q: &ListCatsQuery) -> Vec<Cat> {
        let page = q.page.unwrap_or(1) as usize;
        let limit = q.limit.unwrap_or(10) as usize;
        self.cats
            .read()
            .unwrap()
            .iter()
            .filter(|c| q.breed.as_ref().map(|b| &c.breed == b).unwrap_or(true))
            .skip((page - 1) * limit)
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn find_one(&self, id: u32) -> HttpResult<Cat> {
        self.cats
            .read()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| NotFoundException::new(format!("Cat with id {id} not found")))
    }

    pub fn update(&self, id: u32, dto: UpdateCatDto) -> HttpResult<Cat> {
        let mut cats = self.cats.write().unwrap();
        let cat = cats.iter_mut().find(|c| c.id == id).ok_or_else(|| NotFoundException::new(format!("Cat with id {id} not found")))?;
        if let Some(n) = dto.name {
            cat.name = n;
        }
        if let Some(a) = dto.age {
            cat.age = a;
        }
        if let Some(b) = dto.breed {
            cat.breed = b;
        }
        Ok(cat.clone())
    }

    pub fn remove(&self, id: u32) -> HttpResult<()> {
        let mut cats = self.cats.write().unwrap();
        let before = cats.len();
        cats.retain(|c| c.id != id);
        if cats.len() == before {
            return Err(NotFoundException::new(format!("Cat with id {id} not found")));
        }
        self.logger.log(format!("Cat {id} removed"));
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.cats.read().unwrap().len()
    }
}

#[async_trait]
impl OnModuleInit for CatsService {
    async fn on_module_init(&self) -> corvis::Result<()> {
        self.logger.log("CatsService initialized");
        Ok(())
    }
}
