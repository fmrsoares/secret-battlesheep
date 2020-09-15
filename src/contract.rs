#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]

use cosmwasm_std::{
    generic_err, to_binary, Api, Binary, Env, Extern, HandleResponse, InitResponse, Querier,
    StdResult, Storage,
};

use crate::msg::{Credentials, HandleMsg, InitMsg, QueryMsg};
use crate::state::{Coords, Game, Pasture, Player};

pub fn init<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: InitMsg,
) -> StdResult<InitResponse> {
    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::NewGame { name } => try_new_game(&mut deps.storage, name),
        HandleMsg::Join {
            pasture,
            credentials,
        } => try_join(&mut deps.storage, credentials, pasture),
        HandleMsg::Shoot {
            coords,
            credentials,
        } => try_shoot(&mut deps.storage, credentials, coords),
        HandleMsg::Confirm {
            coords,
            credentials,
        } => try_confirm(&mut deps.storage, credentials, coords),
    }
}

fn try_new_game<S: Storage>(storage: &mut S, name: String) -> StdResult<HandleResponse> {
    // As long as the storage isn't corrupted somehow, this `?` should always succeed.
    if Game::may_load(storage, name.clone())?.is_some() {
        return Err(generic_err(format!(
            "game with name {:?} already exists",
            name
        )));
    }

    Game::new(name).save(storage)?;

    Ok(HandleResponse::default())
}

fn try_join<S: Storage>(
    storage: &mut S,
    credentials: Credentials,
    pasture: Pasture,
) -> StdResult<HandleResponse> {
    let mut game = Game::load(storage, credentials.game.clone())?;
    let player = Player::new(credentials.username, credentials.password, pasture);
    game.add_player(player)?;

    game.save(storage)?;

    Ok(HandleResponse::default())
}

fn try_shoot<S: Storage>(
    storage: &mut S,
    credentials: Credentials,
    coords: Coords,
) -> StdResult<HandleResponse> {
    let mut game = Game::load(storage, credentials.game.clone())?.full()?;

    if game.player().matches_credentials(&credentials) {
        return Err(generic_err("It's not your turn".to_string()));
    }
    game.shoot(coords);

    game.save(storage)?;

    Ok(HandleResponse::default())
}

fn try_confirm<S: Storage>(
    storage: &mut S,
    credentials: Credentials,
    coords: Coords,
) -> StdResult<HandleResponse> {
    let mut game = Game::load(storage, credentials.game.clone())?.full()?;

    if game.opponent().matches_credentials(&credentials) {
        return Err(generic_err(
            "You do not have permissions to confirm this shot".to_string(),
        ));
    }
    game.confirm_shot(coords);
    game.end_turn();

    game.save(storage)?;

    Ok(HandleResponse::default())
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::MyPasture { credentials } => try_get_my_pasture(&deps.storage, credentials),
        QueryMsg::MyShots { credentials } => try_get_my_shots(&deps.storage, credentials),
        QueryMsg::LastShot { credentials } => try_get_last_shot(&deps.storage, credentials),
    }
}

fn try_get_my_pasture<S: Storage>(storage: &S, credentials: Credentials) -> StdResult<Binary> {
    let game = Game::load(storage, credentials.game.clone())?.full()?;

    let pasture = game
        .player()
        .pasture(&credentials)
        .ok_or_else(|| generic_err("You do not have permissions to get the shots".to_string()))?;

    to_binary(pasture)
}

pub fn try_get_my_shots<S: Storage>(storage: &S, credentials: Credentials) -> StdResult<Binary> {
    let game = Game::load(storage, credentials.game.clone())?.full()?;
    let player = game.player();
    let opponent = game.opponent();
    let shots = if player.matches_credentials(&credentials) {
        game.get_player_shots()
    } else if opponent.matches_credentials(&credentials) {
        game.get_opponent_shots()
    } else {
        return Err(generic_err(
            "You do not have permissions to get this information".to_string(),
        ));
    };

    to_binary(&shots)
}

pub fn try_get_last_shot<S: Storage>(storage: &S, credentials: Credentials) -> StdResult<Binary> {
    let game = Game::load(storage, credentials.game.clone())?.full()?;
    let player = game.player();
    let opponent = game.opponent();
    let last_shot =
        if player.matches_credentials(&credentials) || opponent.matches_credentials(&credentials) {
            game.next_shot()
        } else {
            return Err(generic_err(
                "You do not have permissions to get this information".to_string(),
            ));
        };

    to_binary(&last_shot)
}



#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::{coins, StdError, HandleResult, from_binary};
    use crate::state::{Orientation, Herd, Pasture};

    #[test]
    fn main_game_flow() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg { };
        let env = mock_env(&deps.api, "creator", &coins(1000, "token"));

        // we can just call .unwrap() to assert this was a success
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Create new game with "foo" name
        let env = mock_env(&deps.api, "anyone", &coins(2, "token"));
        let msg = HandleMsg::NewGame { name: "foo".to_string() };
        let res: HandleResponse = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Create new game with "foo" name duplicated - ERROR
        let env = mock_env(&deps.api, "anyone", &coins(2, "token"));
        let msg = HandleMsg::NewGame { name: "foo".to_string() };
        let res: HandleResult = handle(&mut deps, env, msg);

        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!(msg, "game with name \"foo\" already exists"),
            e => panic!("Unexpected error: {:?}", e),
        }

        // Player1 - Join the game "foo" with this pasture
        //    | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
        //    |---------------------------------------|
        //  0 | X | x |   |   |   |   | X | x | x | x |
        //  1 |   |   |   |   |   |   |   |   |   |   |
        //  2 |   |   | X | X | x | x |   |   |   |   |
        //  3 |   |   | x |   |   |   |   |   |   |   |
        //  4 |   |   | x |   |   |   |   |   |   |   |
        //  5 |   |   | x |   |   |   |   |   |   |   |
        //  6 |   |   | x |   | X |   |   |   |   |   |
        //  7 |   |   |   |   | x |   |   |   |   |   |
        //  8 |   |   |   |   | x |   |   |   |   |   |
        //  9 |   |   |   |   |   |   |   |   |   |   |
        //    |---------------------------------------|
	    //  2x -> (0,0,Horizontal)
	    //  3x -> (3,2,Horizontal)
	    //  3x -> (4,6,Vertical)
	    //  4x -> (6,0,Horizontal)
	    //  5x -> (2,2,Vertical)

        let msg = HandleMsg::Join { 
            credentials: Credentials {
                game: "foo".to_string(),
                username: "player1".to_string(),
                password: "1111".to_string(),
            }, 
            pasture:Pasture::new(
                vec![
                    // 2x Length Herds
                    Herd::new(0, 0, 2, Orientation::Horizontal),
                    // 3x Length Herds
                    Herd::new(3, 2, 3, Orientation::Horizontal),
                    Herd::new(4, 6, 3, Orientation::Vertical),
                    // 4x Length Herds
                    Herd::new(6, 0, 4, Orientation::Horizontal),
                    // 5x Length Herds
                    Herd::new(2, 2, 5, Orientation::Vertical),
                ],
                vec![]
            )
        };


        let env = mock_env(&deps.api, "anyone", &coins(2, "token"));
        let res: HandleResponse = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Player2 - Join the game "foo" with this pasture
        //    | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
        //    |---------------------------------------|
        //  0 |   |   |   |   |   |   |   |   |   |   |
        //  1 |   |   |   |   |   |   |   |   |   |   |
        //  2 |   |   |   |   |   |   |   |   |   |   |
        //  3 |   |   |   |   |   |   |   |   | X |   |
        //  4 |   | X | x | x | X | x | x |   | x |   |
        //  5 |   |   |   |   |   |   | X |   | x |   |
        //  6 |   |   |   |   |   |   | x |   | x |   |
        //  7 |   |   |   |   |   |   |   |   |   |   |
        //  8 |   |   |   |   |   |   |   |   |   |   |
        //  9 |   |   |   |   |   | X | x | x | x | x |
        //    |---------------------------------------|
	    //  2x -> (6,5,Vertical)
	    //  3x -> (1,4,Horizontal)
	    //  3x -> (4,4,Horizontal)
	    //  4x -> (8,3,Vertical)
        //  5x -> (5,9,Horizontal)
        
        // Other player joins
        let msg = HandleMsg::Join { 
            credentials: Credentials {
                game: "foo".to_string(),
                username: "player2".to_string(),
                password: "2222".to_string(),
            }, 
            pasture:Pasture::new(
                vec![
                    // 2x Length Herds
                    Herd::new(6, 5, 2, Orientation::Vertical),
                    // 3x Length Herds
                    Herd::new(1, 4, 3, Orientation::Horizontal),
                    Herd::new(4, 4, 3, Orientation::Horizontal),
                    // 4x Length Herds
                    Herd::new(8, 3, 4, Orientation::Vertical),
                    // 5x Length Herds
                    Herd::new(5, 9, 5, Orientation::Horizontal),
                ],
                vec![]
            )
        };

        let env = mock_env(&deps.api, "anyone", &coins(2, "token"));
        let res: HandleResponse = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Player 1 - Query pasture created
        let res = query(&mut deps, QueryMsg::MyPasture { 
            credentials: Credentials {
                game: "foo".to_string(),
                username: "player1".to_string(),
                password: "1111".to_string(),
            }
        }).unwrap();
        //println!("{:?}", res.to_base64());
        /*
            {"herds":[{"coords":{"x":0,"y":0},"length":2,"orientation":"horizontal"},{"coords":{"x":3,"y":2},"length":3,"orientation":"horizontal"},{"coords":{"x":4,"y":6},"length":3,"orientation":"vertical"},{"coords":{"x":6,"y":0},"length":4,"orientation":"horizontal"},{"coords":{"x":2,"y":2},"length":5,"orientation":"vertical"}],"shots":[]}
        */
        assert_eq!("eyJoZXJkcyI6W3siY29vcmRzIjp7IngiOjAsInkiOjB9LCJsZW5ndGgiOjIsIm9yaWVudGF0aW9uIjoiaG9yaXpvbnRhbCJ9LHsiY29vcmRzIjp7IngiOjMsInkiOjJ9LCJsZW5ndGgiOjMsIm9yaWVudGF0aW9uIjoiaG9yaXpvbnRhbCJ9LHsiY29vcmRzIjp7IngiOjQsInkiOjZ9LCJsZW5ndGgiOjMsIm9yaWVudGF0aW9uIjoidmVydGljYWwifSx7ImNvb3JkcyI6eyJ4Ijo2LCJ5IjowfSwibGVuZ3RoIjo0LCJvcmllbnRhdGlvbiI6Imhvcml6b250YWwifSx7ImNvb3JkcyI6eyJ4IjoyLCJ5IjoyfSwibGVuZ3RoIjo1LCJvcmllbnRhdGlvbiI6InZlcnRpY2FsIn1dLCJzaG90cyI6W119", res.to_base64());

        // Player 2 - Query pasture created
        let res = query(&mut deps, QueryMsg::MyPasture { 
            credentials: Credentials {
                game: "foo".to_string(),
                username: "player2".to_string(),
                password: "2222".to_string(),
            }
        }).unwrap();
        //println!("{:?}", res.to_base64());
        /*
            {"herds":[{"coords":{"x":0,"y":0},"length":2,"orientation":"horizontal"},{"coords":{"x":3,"y":2},"length":3,"orientation":"horizontal"},{"coords":{"x":4,"y":6},"length":3,"orientation":"vertical"},{"coords":{"x":6,"y":0},"length":4,"orientation":"horizontal"},{"coords":{"x":2,"y":2},"length":5,"orientation":"vertical"}],"shots":[]}
        */
        assert_eq!("eyJoZXJkcyI6W3siY29vcmRzIjp7IngiOjAsInkiOjB9LCJsZW5ndGgiOjIsIm9yaWVudGF0aW9uIjoiaG9yaXpvbnRhbCJ9LHsiY29vcmRzIjp7IngiOjMsInkiOjJ9LCJsZW5ndGgiOjMsIm9yaWVudGF0aW9uIjoiaG9yaXpvbnRhbCJ9LHsiY29vcmRzIjp7IngiOjQsInkiOjZ9LCJsZW5ndGgiOjMsIm9yaWVudGF0aW9uIjoidmVydGljYWwifSx7ImNvb3JkcyI6eyJ4Ijo2LCJ5IjowfSwibGVuZ3RoIjo0LCJvcmllbnRhdGlvbiI6Imhvcml6b250YWwifSx7ImNvb3JkcyI6eyJ4IjoyLCJ5IjoyfSwibGVuZ3RoIjo1LCJvcmllbnRhdGlvbiI6InZlcnRpY2FsIn1dLCJzaG90cyI6W119", res.to_base64());

    }
}